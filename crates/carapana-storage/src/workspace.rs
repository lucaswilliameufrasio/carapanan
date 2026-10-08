use std::{
    error::Error,
    fmt,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_WORKSPACE_FILE_HASH_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMetadata {
    path_bytes: Vec<u8>,
    device: u64,
    inode: u64,
}

/// Metadata and optional bounded SHA-256 evidence for one explicitly selected file.
/// Neither metadata nor the digest authorizes resuming work.
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
    #[serde(default)]
    content_sha256: Option<String>,
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
    SensitivePath(PathBuf),
    FileTooLarge(PathBuf),
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
            Self::SensitivePath(path) => write!(
                formatter,
                "refusing to read a sensitive workspace file for hashing: {}",
                path.display()
            ),
            Self::FileTooLarge(path) => write!(
                formatter,
                "workspace file exceeds the 1 MiB hashing limit: {}",
                path.display()
            ),
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

    /// Hash one explicitly selected, non-sensitive file within the fixed 1 MiB limit.
    pub fn observe_file_with_hash(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<WorkspaceFileMetadata, WorkspaceFileMetadataError> {
        WorkspaceFileMetadata::capture_with_hash(self, relative_path)
    }
}

impl WorkspaceFileMetadata {
    pub(crate) fn validate_path(&self) -> Result<(), WorkspaceFileMetadataError> {
        validate_relative_path(&self.relative_path())
    }

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
            Ok(Self::from_stat(relative_path, &metadata, None))
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(WorkspaceFileMetadataError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }

    pub fn capture_with_hash(
        workspace: &WorkspaceMetadata,
        relative_path: impl AsRef<Path>,
    ) -> Result<Self, WorkspaceFileMetadataError> {
        workspace
            .verify_current()
            .map_err(WorkspaceFileMetadataError::Workspace)?;
        let relative_path = relative_path.as_ref();
        validate_relative_path(relative_path)?;
        let workspace_path = workspace.path();
        if is_sensitive_path(relative_path) || is_sensitive_path(&workspace_path) {
            return Err(WorkspaceFileMetadataError::SensitivePath(
                relative_path.to_owned(),
            ));
        }

        let absolute_path = workspace.path().join(relative_path);
        let path_metadata =
            inspect_path_components(&absolute_path).map_err(map_workspace_file_error)?;
        if !path_metadata.file_type().is_file() {
            return Err(WorkspaceFileMetadataError::NotRegularFile(
                relative_path.to_owned(),
            ));
        }
        if path_metadata.len() > MAX_WORKSPACE_FILE_HASH_BYTES as u64 {
            return Err(WorkspaceFileMetadataError::FileTooLarge(
                relative_path.to_owned(),
            ));
        }

        #[cfg(unix)]
        {
            let mut file = open_workspace_file_no_follow(workspace, relative_path)?;
            let opened_metadata = file.metadata().map_err(WorkspaceFileMetadataError::Io)?;
            if !opened_metadata.file_type().is_file() {
                return Err(WorkspaceFileMetadataError::NotRegularFile(
                    relative_path.to_owned(),
                ));
            }
            if opened_metadata.len() > MAX_WORKSPACE_FILE_HASH_BYTES as u64 {
                return Err(WorkspaceFileMetadataError::FileTooLarge(
                    relative_path.to_owned(),
                ));
            }

            let path_observation = Self::from_stat(relative_path, &path_metadata, None);
            let opened_observation = Self::from_stat(relative_path, &opened_metadata, None);
            if path_observation != opened_observation {
                return Err(WorkspaceFileMetadataError::Changed(
                    relative_path.to_owned(),
                ));
            }

            let mut hasher = Sha256::new();
            let mut total_read = 0_usize;
            let mut buffer = [0_u8; 8192];
            loop {
                let remaining = MAX_WORKSPACE_FILE_HASH_BYTES.saturating_sub(total_read);
                if remaining == 0 {
                    break;
                }
                let read_length = remaining.min(buffer.len());
                let bytes_read = file
                    .read(&mut buffer[..read_length])
                    .map_err(WorkspaceFileMetadataError::Io)?;
                if bytes_read == 0 {
                    break;
                }
                total_read = total_read.saturating_add(bytes_read);
                hasher.update(&buffer[..bytes_read]);
            }

            let final_metadata = file.metadata().map_err(WorkspaceFileMetadataError::Io)?;
            if final_metadata.len() > MAX_WORKSPACE_FILE_HASH_BYTES as u64 {
                return Err(WorkspaceFileMetadataError::FileTooLarge(
                    relative_path.to_owned(),
                ));
            }
            let final_observation = Self::from_stat(relative_path, &final_metadata, None);
            if opened_observation != final_observation || total_read as u64 != final_metadata.len()
            {
                return Err(WorkspaceFileMetadataError::Changed(
                    relative_path.to_owned(),
                ));
            }

            let digest = format!("{:x}", hasher.finalize());
            Ok(Self::from_stat(
                relative_path,
                &final_metadata,
                Some(digest),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = path_metadata;
            Err(WorkspaceFileMetadataError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }

    /// Revalidate the stored stat fields and, when present, the bounded content digest.
    pub fn verify_current(
        &self,
        workspace: &WorkspaceMetadata,
    ) -> Result<(), WorkspaceFileMetadataError> {
        let relative_path = self.relative_path();
        let current = if self.content_sha256.is_some() {
            Self::capture_with_hash(workspace, &relative_path)?
        } else {
            Self::capture(workspace, &relative_path)?
        };
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

    pub fn content_sha256(&self) -> Option<&str> {
        self.content_sha256.as_deref()
    }

    #[cfg(unix)]
    fn from_stat(
        relative_path: &Path,
        metadata: &std::fs::Metadata,
        content_sha256: Option<String>,
    ) -> Self {
        use std::{os::unix::ffi::OsStrExt, os::unix::fs::MetadataExt};

        Self {
            relative_path_bytes: relative_path.as_os_str().as_bytes().to_vec(),
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
            content_sha256,
        }
    }
}

fn is_sensitive_path(path: &Path) -> bool {
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = name.to_string_lossy().to_lowercase();
        if name == ".ssh"
            || name == ".aws"
            || name.starts_with(".env")
            || name.contains("secret")
            || name.contains("credential")
        {
            return true;
        }
    }

    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase());
    matches!(extension.as_deref(), Some("pem" | "key"))
}

#[cfg(unix)]
fn open_workspace_file_no_follow(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
) -> Result<std::fs::File, WorkspaceFileMetadataError> {
    use rustix::{
        fd::AsFd,
        fs::{Mode, OFlags, openat},
    };
    use std::os::unix::fs::MetadataExt;

    let mut current = std::fs::File::open("/").map_err(WorkspaceFileMetadataError::Io)?;
    let workspace_path = workspace.path();
    for component in workspace_path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_child_directory(&current, name)
                    .map_err(|error| map_open_error(error, &workspace_path))?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(WorkspaceFileMetadataError::UnsafePath(workspace_path));
            }
        }
    }

    let metadata = current.metadata().map_err(WorkspaceFileMetadataError::Io)?;
    if metadata.dev() != workspace.device || metadata.ino() != workspace.inode {
        return Err(WorkspaceFileMetadataError::Workspace(
            WorkspaceMetadataError::IdentityChanged(workspace_path),
        ));
    }

    let components: Vec<_> = relative_path.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(WorkspaceFileMetadataError::InvalidRelativePath(
                relative_path.to_owned(),
            ));
        };
        if index + 1 == components.len() {
            let descriptor = openat(
                current.as_fd(),
                *name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| map_open_error(error, relative_path))?;
            return Ok(std::fs::File::from(descriptor));
        }
        current = open_child_directory(&current, name)
            .map_err(|error| map_open_error(error, relative_path))?;
    }

    Err(WorkspaceFileMetadataError::InvalidRelativePath(
        relative_path.to_owned(),
    ))
}

#[cfg(unix)]
fn open_child_directory(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
) -> Result<std::fs::File, rustix::io::Errno> {
    use rustix::{
        fd::AsFd,
        fs::{Mode, OFlags, openat},
    };

    let descriptor = openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    Ok(std::fs::File::from(descriptor))
}

#[cfg(unix)]
fn map_open_error(error: rustix::io::Errno, path: &Path) -> WorkspaceFileMetadataError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        WorkspaceFileMetadataError::UnsafePath(path.to_owned())
    } else {
        let error = io::Error::from(error);
        WorkspaceFileMetadataError::Io(io::Error::new(
            error.kind(),
            format!("{}: {error}", path.display()),
        ))
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
    fn should_hash_an_explicit_file_and_reject_content_changes() {
        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let file = workspace.join("src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"abc").unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();

        let observation = workspace_metadata
            .observe_file_with_hash("src/main.rs")
            .unwrap();
        assert_eq!(
            observation.content_sha256(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        observation.verify_current(&workspace_metadata).unwrap();

        fs::write(&file, b"xyz").unwrap();
        assert!(matches!(
            observation.verify_current(&workspace_metadata),
            Err(WorkspaceFileMetadataError::Changed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_deny_sensitive_file_names_and_files_over_the_hash_limit() {
        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join(".env.local"), b"not read by the validator").unwrap();
        fs::create_dir_all(workspace.join(".ssh")).unwrap();
        fs::create_dir_all(workspace.join(".aws")).unwrap();
        fs::write(workspace.join(".ssh/id_rsa"), b"private test data").unwrap();
        fs::write(workspace.join(".aws/credentials"), b"private test data").unwrap();
        fs::write(workspace.join("my-secret-notes.txt"), b"private test data").unwrap();
        fs::write(workspace.join("service.pem"), b"private test data").unwrap();
        fs::write(workspace.join("service.key"), b"private test data").unwrap();
        fs::write(
            workspace.join("notes.txt"),
            vec![b'x'; super::MAX_WORKSPACE_FILE_HASH_BYTES + 1],
        )
        .unwrap();
        fs::write(
            workspace.join("at-limit.txt"),
            vec![b'x'; super::MAX_WORKSPACE_FILE_HASH_BYTES],
        )
        .unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();

        for sensitive_path in [
            ".env.local",
            ".ssh/id_rsa",
            ".aws/credentials",
            "my-secret-notes.txt",
            "service.pem",
            "service.key",
        ] {
            assert!(matches!(
                workspace_metadata.observe_file_with_hash(sensitive_path),
                Err(WorkspaceFileMetadataError::SensitivePath(_))
            ));
        }
        assert!(matches!(
            workspace_metadata.observe_file_with_hash("notes.txt"),
            Err(WorkspaceFileMetadataError::FileTooLarge(_))
        ));
        assert!(
            workspace_metadata
                .observe_file_with_hash("at-limit.txt")
                .is_ok()
        );
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
        assert!(matches!(
            workspace_metadata.observe_file_with_hash("linked.txt"),
            Err(WorkspaceFileMetadataError::UnsafePath(_))
        ));
    }
}
