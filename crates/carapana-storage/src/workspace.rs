use std::{
    error::Error,
    fmt, io,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMetadata {
    path_bytes: Vec<u8>,
    device: u64,
    inode: u64,
}

/// Stat-only evidence for one explicitly selected regular file under a workspace.
/// It does not prove content is unchanged and cannot authorize resuming work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFileMetadata {
    relative_path_bytes: Vec<u8>,
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Debug)]
pub enum WorkspaceMetadataError {
    UnsupportedPlatform,
    InvalidPath(PathBuf),
    UnsafePath(PathBuf),
    IdentityChanged(PathBuf),
    Io(io::Error),
}

#[derive(Debug)]
pub enum WorkspaceFileMetadataError {
    Workspace(WorkspaceMetadataError),
    InvalidRelativePath(PathBuf),
    UnsafePath(PathBuf),
    NotRegularFile(PathBuf),
    Changed(PathBuf),
    Io(io::Error),
}

impl fmt::Display for WorkspaceMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("workspace identity is supported on Unix platforms only")
            }
            Self::InvalidPath(path) => {
                write!(
                    formatter,
                    "workspace path must be absolute without parent traversal: {}",
                    path.display()
                )
            }
            Self::UnsafePath(path) => write!(
                formatter,
                "refusing workspace path with a symlink or non-directory component: {}",
                path.display()
            ),
            Self::IdentityChanged(path) => write!(
                formatter,
                "workspace directory identity changed since it was recorded: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "workspace metadata check failed: {error}"),
        }
    }
}

impl Error for WorkspaceMetadataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for WorkspaceFileMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(formatter, "workspace validation failed: {error}"),
            Self::InvalidRelativePath(path) => write!(
                formatter,
                "workspace file path must be relative and stay within the workspace: {}",
                path.display()
            ),
            Self::UnsafePath(path) => write!(
                formatter,
                "refusing workspace file path with a symlink or non-directory component: {}",
                path.display()
            ),
            Self::NotRegularFile(path) => {
                write!(
                    formatter,
                    "workspace path is not a regular file: {}",
                    path.display()
                )
            }
            Self::Changed(path) => write!(
                formatter,
                "workspace file metadata changed since observation: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "workspace file metadata check failed: {error}"),
        }
    }
}

impl Error for WorkspaceFileMetadataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl WorkspaceMetadata {
    /// Capture the opened folder boundary without canonicalizing or following symlinks.
    pub fn capture(path: impl AsRef<Path>) -> Result<Self, WorkspaceMetadataError> {
        let path = path.as_ref();
        let (device, inode) = directory_identity(path)?;
        #[cfg(unix)]
        let path_bytes = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let path_bytes = {
            let _ = path;
            return Err(WorkspaceMetadataError::UnsupportedPlatform);
        };

        Ok(Self {
            path_bytes,
            device,
            inode,
        })
    }

    /// Re-check the same path and filesystem identity without changing the workspace.
    pub fn verify_current(&self) -> Result<(), WorkspaceMetadataError> {
        let path = self.path();
        let (device, inode) = directory_identity(&path)?;
        if device != self.device || inode != self.inode {
            return Err(WorkspaceMetadataError::IdentityChanged(path));
        }
        Ok(())
    }

    pub fn path(&self) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(self.path_bytes.clone()))
        }
        #[cfg(not(unix))]
        {
            PathBuf::new()
        }
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }

    /// Capture stat metadata for one explicitly selected file without opening or reading it.
    pub fn observe_file(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<WorkspaceFileMetadata, WorkspaceFileMetadataError> {
        WorkspaceFileMetadata::capture(self, relative_path)
    }
}

impl WorkspaceFileMetadata {
    pub fn capture(
        workspace: &WorkspaceMetadata,
        relative_path: impl AsRef<Path>,
    ) -> Result<Self, WorkspaceFileMetadataError> {
        workspace
            .verify_current()
            .map_err(WorkspaceFileMetadataError::Workspace)?;
        let relative_path = relative_path.as_ref();
        validate_relative_path(relative_path)?;
        let absolute_path = workspace.path().join(relative_path);
        let metadata = inspect_path_components(&absolute_path).map_err(map_workspace_file_error)?;
        if !metadata.file_type().is_file() {
            return Err(WorkspaceFileMetadataError::NotRegularFile(
                relative_path.to_owned(),
            ));
        }

        #[cfg(unix)]
        {
            use std::{os::unix::ffi::OsStrExt, os::unix::fs::MetadataExt};
            Ok(Self {
                relative_path_bytes: relative_path.as_os_str().as_bytes().to_vec(),
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                size: metadata.len(),
                modified_seconds: metadata.mtime(),
                modified_nanoseconds: metadata.mtime_nsec(),
                changed_seconds: metadata.ctime(),
                changed_nanoseconds: metadata.ctime_nsec(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(WorkspaceFileMetadataError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }

    /// Compare the current stat fields only; content changes that preserve them are invisible.
    pub fn verify_current(
        &self,
        workspace: &WorkspaceMetadata,
    ) -> Result<(), WorkspaceFileMetadataError> {
        let relative_path = self.relative_path();
        let current = Self::capture(workspace, &relative_path)?;
        if current != *self {
            return Err(WorkspaceFileMetadataError::Changed(relative_path));
        }
        Ok(())
    }

    pub fn relative_path(&self) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(
                self.relative_path_bytes.clone(),
            ))
        }
        #[cfg(not(unix))]
        {
            PathBuf::new()
        }
    }
}

fn validate_relative_path(path: &Path) -> Result<(), WorkspaceFileMetadataError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(WorkspaceFileMetadataError::InvalidRelativePath(
            path.to_owned(),
        ));
    }

    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(WorkspaceFileMetadataError::InvalidRelativePath(
                    path.to_owned(),
                ));
            }
        }
    }
    if !has_normal_component {
        return Err(WorkspaceFileMetadataError::InvalidRelativePath(
            path.to_owned(),
        ));
    }
    Ok(())
}

fn map_workspace_file_error(error: WorkspaceMetadataError) -> WorkspaceFileMetadataError {
    match error {
        WorkspaceMetadataError::InvalidPath(path) | WorkspaceMetadataError::UnsafePath(path) => {
            WorkspaceFileMetadataError::UnsafePath(path)
        }
        WorkspaceMetadataError::IdentityChanged(path) => {
            WorkspaceFileMetadataError::Workspace(WorkspaceMetadataError::IdentityChanged(path))
        }
        WorkspaceMetadataError::Io(error) => WorkspaceFileMetadataError::Io(error),
        WorkspaceMetadataError::UnsupportedPlatform => {
            WorkspaceFileMetadataError::Workspace(WorkspaceMetadataError::UnsupportedPlatform)
        }
    }
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Result<(u64, u64), WorkspaceMetadataError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = inspect_path_components(path)?;
    if !metadata.file_type().is_dir() {
        return Err(WorkspaceMetadataError::UnsafePath(path.to_owned()));
    }
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
fn inspect_path_components(path: &Path) -> Result<std::fs::Metadata, WorkspaceMetadataError> {
    if !path.is_absolute() {
        return Err(WorkspaceMetadataError::InvalidPath(path.to_owned()));
    }

    let mut component_path = PathBuf::new();
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => component_path.push(component.as_os_str()),
            Component::Normal(name) => component_path.push(name),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(WorkspaceMetadataError::InvalidPath(path.to_owned()));
            }
        }
        components.push(component_path.clone());
    }

    if components.is_empty() {
        return Err(WorkspaceMetadataError::InvalidPath(path.to_owned()));
    }

    let mut final_metadata = None;
    for (index, component_path) in components.iter().enumerate() {
        let metadata =
            std::fs::symlink_metadata(component_path).map_err(WorkspaceMetadataError::Io)?;
        if metadata.file_type().is_symlink()
            || (index + 1 < components.len() && !metadata.file_type().is_dir())
        {
            return Err(WorkspaceMetadataError::UnsafePath(component_path.clone()));
        }
        final_metadata = Some(metadata);
    }

    final_metadata.ok_or_else(|| WorkspaceMetadataError::InvalidPath(path.into()))
}

#[cfg(not(unix))]
fn inspect_path_components(path: &Path) -> Result<std::fs::Metadata, WorkspaceMetadataError> {
    let _ = path;
    Err(WorkspaceMetadataError::UnsupportedPlatform)
}

#[cfg(not(unix))]
fn directory_identity(path: &Path) -> Result<(u64, u64), WorkspaceMetadataError> {
    let _ = path;
    Err(WorkspaceMetadataError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceFileMetadataError, WorkspaceMetadata, WorkspaceMetadataError};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-workspace-metadata-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn should_capture_and_verify_directory_identity_without_changing_the_path() {
        use std::os::unix::fs::MetadataExt;

        let directory = TestDirectory::new();
        let metadata = WorkspaceMetadata::capture(&directory.0).unwrap();
        let filesystem_metadata = fs::metadata(&directory.0).unwrap();

        assert_eq!(metadata.path(), directory.0);
        assert_eq!(metadata.device(), filesystem_metadata.dev());
        assert_eq!(metadata.inode(), filesystem_metadata.ino());
        let encoded = serde_json::to_vec(&metadata).unwrap();
        let decoded: WorkspaceMetadata = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, metadata);
        metadata.verify_current().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_relative_and_symlinked_workspace_paths() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        assert!(matches!(
            WorkspaceMetadata::capture("relative/workspace"),
            Err(WorkspaceMetadataError::InvalidPath(_))
        ));

        let external = directory.0.join("external");
        fs::create_dir(&external).unwrap();
        let linked = directory.0.join("linked");
        symlink(&external, &linked).unwrap();
        assert!(matches!(
            WorkspaceMetadata::capture(&linked),
            Err(WorkspaceMetadataError::UnsafePath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_detect_a_workspace_directory_replaced_at_the_same_path() {
        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        let moved = directory.0.join("moved-workspace");
        fs::create_dir(&workspace).unwrap();
        let metadata = WorkspaceMetadata::capture(&workspace).unwrap();
        fs::rename(&workspace, &moved).unwrap();
        fs::create_dir(&workspace).unwrap();

        assert!(matches!(
            metadata.verify_current(),
            Err(WorkspaceMetadataError::IdentityChanged(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_capture_and_verify_stat_metadata_for_an_explicit_workspace_file() {
        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let file = workspace.join("src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"contents are not stored in metadata").unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();

        let observation = workspace_metadata.observe_file("src/main.rs").unwrap();
        observation.verify_current(&workspace_metadata).unwrap();
        assert_eq!(observation.relative_path(), PathBuf::from("src/main.rs"));
        let encoded = serde_json::to_vec(&observation).unwrap();
        assert!(!String::from_utf8_lossy(&encoded).contains("contents are not stored in metadata"));
        let decoded: super::WorkspaceFileMetadata = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, observation);
    }

    #[cfg(unix)]
    #[test]
    fn should_detect_workspace_file_permission_changes_and_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let file = workspace.join("main.rs");
        fs::write(&file, b"baseline").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();
        let observation = workspace_metadata.observe_file("main.rs").unwrap();

        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            observation.verify_current(&workspace_metadata),
            Err(WorkspaceFileMetadataError::Changed(_))
        ));

        let moved_file = workspace.join("main.rs.original");
        fs::rename(&file, &moved_file).unwrap();
        fs::write(&file, b"replacement").unwrap();
        assert!(matches!(
            observation.verify_current(&workspace_metadata),
            Err(WorkspaceFileMetadataError::Changed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_workspace_file_escape_and_symlink_paths() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let file = workspace.join("inside.txt");
        fs::write(&file, b"local content").unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();

        for invalid_path in [PathBuf::from("../outside.txt"), file.clone()] {
            assert!(matches!(
                workspace_metadata.observe_file(invalid_path),
                Err(WorkspaceFileMetadataError::InvalidRelativePath(_))
            ));
        }

        let linked_file = workspace.join("linked.txt");
        symlink(&file, &linked_file).unwrap();
        assert!(matches!(
            workspace_metadata.observe_file("linked.txt"),
            Err(WorkspaceFileMetadataError::UnsafePath(_))
        ));
    }
}
