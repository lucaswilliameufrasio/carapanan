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

#[derive(Debug)]
pub enum WorkspaceMetadataError {
    UnsupportedPlatform,
    InvalidPath(PathBuf),
    UnsafePath(PathBuf),
    IdentityChanged(PathBuf),
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
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Result<(u64, u64), WorkspaceMetadataError> {
    use std::os::unix::fs::MetadataExt;

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

    let metadata =
        final_metadata.ok_or_else(|| WorkspaceMetadataError::InvalidPath(path.into()))?;
    if !metadata.file_type().is_dir() {
        return Err(WorkspaceMetadataError::UnsafePath(path.to_owned()));
    }
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn directory_identity(path: &Path) -> Result<(u64, u64), WorkspaceMetadataError> {
    let _ = path;
    Err(WorkspaceMetadataError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceMetadata, WorkspaceMetadataError};
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
}
