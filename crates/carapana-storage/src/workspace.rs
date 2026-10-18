use std::{
    error::Error,
    ffi::OsString,
    fmt,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod write;

pub use write::{MAX_WORKSPACE_FILE_WRITE_BYTES, WorkspaceFileSnapshot, WorkspaceFileWriteError};

pub const MAX_WORKSPACE_FILE_HASH_BYTES: usize = 1024 * 1024;
pub const MAX_WORKSPACE_FILE_READ_BYTES: usize = 256 * 1024;
pub const MAX_WORKSPACE_DIRECTORY_READ_ENTRIES: usize = 50_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMetadata {
    path_bytes: Vec<u8>,
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceDirectoryEntryKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDirectoryEntry {
    pub name: OsString,
    pub kind: WorkspaceDirectoryEntryKind,
    identity: WorkspaceDirectoryIdentity,
}

impl WorkspaceDirectoryEntry {
    pub fn identity(&self) -> &WorkspaceDirectoryIdentity {
        &self.identity
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDirectoryListing {
    pub entries: Vec<WorkspaceDirectoryEntry>,
    pub entries_seen: usize,
    pub sensitive_entries_skipped: usize,
    pub truncated: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFileValidationStatus {
    Unchanged,
    Changed,
    Missing,
    Unreadable,
    Unsafe,
    TooLarge,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFileValidation {
    pub relative_path: PathBuf,
    pub status: WorkspaceFileValidationStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceValidationReport {
    pub files: Vec<WorkspaceFileValidation>,
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

#[derive(Debug)]
pub enum WorkspaceFileReadError {
    Workspace(WorkspaceMetadataError),
    InvalidRelativePath(PathBuf),
    UnsafePath(PathBuf),
    SensitivePath(PathBuf),
    NotRegularFile(PathBuf),
    FileTooLarge(PathBuf),
    MetadataTooLarge(PathBuf),
    Changed(PathBuf),
    Io(io::Error),
}

#[derive(Debug)]
pub enum WorkspaceDirectoryReadError {
    Workspace(WorkspaceMetadataError),
    InvalidRelativePath(PathBuf),
    UnsafePath(PathBuf),
    SensitivePath(PathBuf),
    NotDirectory(PathBuf),
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

impl fmt::Display for WorkspaceFileReadError {
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
            Self::SensitivePath(path) => write!(
                formatter,
                "refusing to read a sensitive workspace file: {}",
                path.display()
            ),
            Self::NotRegularFile(path) => {
                write!(
                    formatter,
                    "workspace path is not a regular file: {}",
                    path.display()
                )
            }
            Self::FileTooLarge(path) => write!(
                formatter,
                "workspace file exceeds the configured read limit (maximum 256 KiB): {}",
                path.display()
            ),
            Self::MetadataTooLarge(path) => write!(
                formatter,
                "workspace file metadata exceeds the configured snapshot limit: {}",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "workspace file changed while it was being read: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "workspace file read failed: {error}"),
        }
    }
}

impl Error for WorkspaceFileReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for WorkspaceDirectoryReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(formatter, "workspace validation failed: {error}"),
            Self::InvalidRelativePath(path) => write!(
                formatter,
                "workspace directory path must be relative and stay within the workspace: {}",
                path.display()
            ),
            Self::UnsafePath(path) => write!(
                formatter,
                "refusing workspace directory path with a symlink or non-directory component: {}",
                path.display()
            ),
            Self::SensitivePath(path) => write!(
                formatter,
                "refusing to inspect a sensitive workspace directory: {}",
                path.display()
            ),
            Self::NotDirectory(path) => {
                write!(
                    formatter,
                    "workspace path is not a directory: {}",
                    path.display()
                )
            }
            Self::Changed(path) => write!(
                formatter,
                "workspace directory changed while it was being read: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "workspace directory read failed: {error}"),
        }
    }
}

impl Error for WorkspaceDirectoryReadError {
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

    pub fn directory_identity(&self) -> WorkspaceDirectoryIdentity {
        WorkspaceDirectoryIdentity {
            device: self.device,
            inode: self.inode,
        }
    }

    /// Apply the conservative filename-based secret denylist to one relative path.
    /// This is a filter hint, not authorization or content-level secret detection.
    pub fn is_sensitive_relative_path(&self, relative_path: impl AsRef<Path>) -> bool {
        is_sensitive_path(relative_path.as_ref()) || is_sensitive_path(&self.path())
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

    /// Read one regular, non-sensitive workspace file without following symlinks.
    /// The read is bounded and rejected if the file changes while it is open.
    pub fn read_file(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<Vec<u8>, WorkspaceFileReadError> {
        self.read_file_with_limit(relative_path, MAX_WORKSPACE_FILE_READ_BYTES)
    }

    /// Read one regular, non-sensitive file with a caller-selected bound that cannot exceed
    /// the workspace-wide 256 KiB maximum.
    pub fn read_file_with_limit(
        &self,
        relative_path: impl AsRef<Path>,
        requested_limit: usize,
    ) -> Result<Vec<u8>, WorkspaceFileReadError> {
        let read_limit = requested_limit.min(MAX_WORKSPACE_FILE_READ_BYTES);
        let read_limit_bytes = u64::try_from(read_limit).unwrap_or(u64::MAX);
        self.verify_current()
            .map_err(WorkspaceFileReadError::Workspace)?;
        let relative_path = relative_path.as_ref();
        validate_relative_path(relative_path)
            .map_err(|_| WorkspaceFileReadError::InvalidRelativePath(relative_path.to_owned()))?;
        let workspace_path = self.path();
        if self.is_sensitive_relative_path(relative_path) {
            return Err(WorkspaceFileReadError::SensitivePath(
                relative_path.to_owned(),
            ));
        }

        let absolute_path = workspace_path.join(relative_path);
        let path_metadata = inspect_path_components(&absolute_path)
            .map_err(map_workspace_file_read_workspace_error)?;
        if !path_metadata.file_type().is_file() {
            return Err(WorkspaceFileReadError::NotRegularFile(
                relative_path.to_owned(),
            ));
        }
        if path_metadata.len() > read_limit_bytes {
            return Err(WorkspaceFileReadError::FileTooLarge(
                relative_path.to_owned(),
            ));
        }

        #[cfg(unix)]
        {
            let mut file = open_workspace_file_no_follow(self, relative_path)
                .map_err(map_workspace_file_read_metadata_error)?;
            let opened_metadata = file.metadata().map_err(WorkspaceFileReadError::Io)?;
            if !opened_metadata.file_type().is_file() {
                return Err(WorkspaceFileReadError::NotRegularFile(
                    relative_path.to_owned(),
                ));
            }
            if opened_metadata.len() > read_limit_bytes {
                return Err(WorkspaceFileReadError::FileTooLarge(
                    relative_path.to_owned(),
                ));
            }

            let path_observation =
                WorkspaceFileMetadata::from_stat(relative_path, &path_metadata, None);
            let opened_observation =
                WorkspaceFileMetadata::from_stat(relative_path, &opened_metadata, None);
            if path_observation != opened_observation {
                return Err(WorkspaceFileReadError::Changed(relative_path.to_owned()));
            }

            let mut contents = Vec::with_capacity(opened_metadata.len() as usize);
            file.by_ref()
                .take(read_limit_bytes)
                .read_to_end(&mut contents)
                .map_err(WorkspaceFileReadError::Io)?;
            if contents.len() > read_limit {
                return Err(WorkspaceFileReadError::FileTooLarge(
                    relative_path.to_owned(),
                ));
            }

            let final_metadata = file.metadata().map_err(WorkspaceFileReadError::Io)?;
            let final_observation =
                WorkspaceFileMetadata::from_stat(relative_path, &final_metadata, None);
            if opened_observation != final_observation
                || contents.len() as u64 != final_metadata.len()
            {
                return Err(WorkspaceFileReadError::Changed(relative_path.to_owned()));
            }

            Ok(contents)
        }
        #[cfg(not(unix))]
        {
            let _ = path_metadata;
            Err(WorkspaceFileReadError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }

    /// Read direct entry metadata from one directory opened relative to the captured workspace.
    ///
    /// An empty relative path selects the workspace root. `expected_identity` must be the identity
    /// recorded when the directory was discovered; a replacement fails closed. Symlinks are
    /// reported as metadata and never followed. The directory is revalidated after enumeration,
    /// and entries on a different filesystem are omitted. Sensitive names are omitted and counted.
    pub fn read_directory_with_limit(
        &self,
        relative_path: impl AsRef<Path>,
        expected_identity: &WorkspaceDirectoryIdentity,
        requested_entry_limit: usize,
    ) -> Result<WorkspaceDirectoryListing, WorkspaceDirectoryReadError> {
        let relative_path = relative_path.as_ref();
        if !relative_path.as_os_str().is_empty() {
            validate_relative_path(relative_path).map_err(|_| {
                WorkspaceDirectoryReadError::InvalidRelativePath(relative_path.to_owned())
            })?;
        }
        if self.is_sensitive_relative_path(relative_path) {
            return Err(WorkspaceDirectoryReadError::SensitivePath(
                relative_path.to_owned(),
            ));
        }

        let entry_limit = requested_entry_limit.min(MAX_WORKSPACE_DIRECTORY_READ_ENTRIES);
        #[cfg(unix)]
        {
            read_workspace_directory_no_follow(self, relative_path, expected_identity, entry_limit)
        }
        #[cfg(not(unix))]
        {
            let _ = (expected_identity, entry_limit);
            Err(WorkspaceDirectoryReadError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
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

    pub fn size_bytes(&self) -> u64 {
        self.size
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
            || name == ".kube"
            || name.starts_with(".env")
            || name.contains("secret")
            || name.contains("credential")
            || matches!(
                name.as_str(),
                ".npmrc" | ".pypirc" | ".netrc" | "id_rsa" | "id_ed25519" | "kubeconfig"
            )
            || name.ends_with(".tfstate")
            || name.ends_with(".tfstate.backup")
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
fn read_workspace_directory_no_follow(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
    expected_identity: &WorkspaceDirectoryIdentity,
    entry_limit: usize,
) -> Result<WorkspaceDirectoryListing, WorkspaceDirectoryReadError> {
    use rustix::{
        fd::AsFd,
        fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat},
    };
    use std::{
        ffi::OsStr,
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
    };

    let opened_chain = open_workspace_directory_chain_no_follow(workspace, relative_path)?;
    let opened_signatures = directory_chain_signatures(&opened_chain)?;
    let directory = opened_chain
        .last()
        .ok_or_else(|| WorkspaceDirectoryReadError::Changed(relative_path.to_owned()))?;
    let opened_metadata = directory
        .metadata()
        .map_err(WorkspaceDirectoryReadError::Io)?;
    let opened_identity = WorkspaceDirectoryIdentity {
        device: opened_metadata.dev(),
        inode: opened_metadata.ino(),
    };
    if opened_identity != *expected_identity {
        return Err(WorkspaceDirectoryReadError::Changed(
            relative_path.to_owned(),
        ));
    }
    if !opened_metadata.is_dir() {
        return Err(WorkspaceDirectoryReadError::NotDirectory(
            relative_path.to_owned(),
        ));
    }

    let directory_stream_fd = openat(
        directory.as_fd(),
        OsStr::new("."),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| map_directory_open_error(error, relative_path))?;
    let mut directory_stream = Dir::new(directory_stream_fd)
        .map_err(|error| WorkspaceDirectoryReadError::Io(io::Error::from(error)))?;
    let mut entries = Vec::new();
    let mut sensitive_entries_skipped = 0_usize;
    let mut entries_seen = 0_usize;
    let mut truncated = false;

    while let Some(entry_result) = directory_stream.read() {
        let entry = entry_result
            .map_err(|error| WorkspaceDirectoryReadError::Io(io::Error::from(error)))?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes()).to_os_string();
        if name == "." || name == ".." {
            continue;
        }

        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > entry_limit {
            truncated = true;
            break;
        }

        let child_relative_path = relative_path.join(&name);
        if workspace.is_sensitive_relative_path(&child_relative_path) {
            sensitive_entries_skipped = sensitive_entries_skipped.saturating_add(1);
            continue;
        }

        let stat = statat(
            directory_stream
                .fd()
                .map_err(|error| WorkspaceDirectoryReadError::Io(io::Error::from(error)))?,
            entry.file_name(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|error| map_directory_open_error(error, &child_relative_path))?;
        #[cfg(target_os = "macos")]
        let device = u64::try_from(stat.st_dev).map_err(|_| {
            WorkspaceDirectoryReadError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem returned an invalid device identifier",
            ))
        })?;
        #[cfg(not(target_os = "macos"))]
        let device = stat.st_dev;
        if device != workspace.device {
            continue;
        }

        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => WorkspaceDirectoryEntryKind::RegularFile,
            FileType::Directory => WorkspaceDirectoryEntryKind::Directory,
            FileType::Symlink => WorkspaceDirectoryEntryKind::Symlink,
            _ => WorkspaceDirectoryEntryKind::Other,
        };
        entries.push(WorkspaceDirectoryEntry {
            name,
            kind,
            identity: WorkspaceDirectoryIdentity {
                device,
                inode: stat.st_ino,
            },
        });
    }

    verify_workspace_directory_chain(workspace, relative_path, &opened_chain, &opened_signatures)?;

    Ok(WorkspaceDirectoryListing {
        entries,
        entries_seen,
        sensitive_entries_skipped,
        truncated,
    })
}

#[cfg(unix)]
fn open_workspace_directory_chain_no_follow(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
) -> Result<Vec<std::fs::File>, WorkspaceDirectoryReadError> {
    use std::os::unix::fs::MetadataExt;

    let workspace_path = workspace.path();
    let mut current = std::fs::File::open("/").map_err(WorkspaceDirectoryReadError::Io)?;
    for component in workspace_path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_child_directory(&current, name)
                    .map_err(|error| map_directory_open_error(error, &workspace_path))?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(WorkspaceDirectoryReadError::Workspace(
                    WorkspaceMetadataError::InvalidPath(workspace_path),
                ));
            }
        }
    }

    let root_metadata = current
        .metadata()
        .map_err(WorkspaceDirectoryReadError::Io)?;
    if !root_metadata.is_dir() {
        return Err(WorkspaceDirectoryReadError::UnsafePath(workspace_path));
    }
    if root_metadata.dev() != workspace.device || root_metadata.ino() != workspace.inode {
        return Err(WorkspaceDirectoryReadError::Workspace(
            WorkspaceMetadataError::IdentityChanged(workspace_path),
        ));
    }
    let mut directory_chain = vec![current];

    for component in relative_path.components() {
        let Component::Normal(name) = component else {
            return Err(WorkspaceDirectoryReadError::InvalidRelativePath(
                relative_path.to_owned(),
            ));
        };
        let parent = directory_chain
            .last()
            .ok_or_else(|| WorkspaceDirectoryReadError::Changed(relative_path.to_owned()))?;
        let child = open_child_directory(parent, name)
            .map_err(|error| map_directory_open_error(error, relative_path))?;
        directory_chain.push(child);
    }

    let directory = directory_chain
        .last()
        .ok_or_else(|| WorkspaceDirectoryReadError::Changed(relative_path.to_owned()))?;
    let metadata = directory
        .metadata()
        .map_err(WorkspaceDirectoryReadError::Io)?;
    if !metadata.is_dir() {
        return Err(WorkspaceDirectoryReadError::NotDirectory(
            relative_path.to_owned(),
        ));
    }
    // Only directories inside the selected workspace participate in mutation checks. Watching
    // ancestors such as /tmp would make unrelated workspaces invalidate this traversal.
    Ok(directory_chain)
}

#[cfg(unix)]
type DirectoryMetadataSignature = (u64, u64, u32, u64, i64, i64, i64, i64);

#[cfg(unix)]
fn directory_metadata_signature(metadata: &std::fs::Metadata) -> DirectoryMetadataSignature {
    use std::os::unix::fs::MetadataExt;

    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[cfg(unix)]
fn directory_chain_signatures(
    directories: &[std::fs::File],
) -> Result<Vec<DirectoryMetadataSignature>, WorkspaceDirectoryReadError> {
    let mut signatures = Vec::with_capacity(directories.len());
    for directory in directories {
        let metadata = directory
            .metadata()
            .map_err(WorkspaceDirectoryReadError::Io)?;
        signatures.push(directory_metadata_signature(&metadata));
    }
    Ok(signatures)
}

#[cfg(unix)]
fn verify_workspace_directory_chain(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
    opened_chain: &[std::fs::File],
    opened_signatures: &[DirectoryMetadataSignature],
) -> Result<(), WorkspaceDirectoryReadError> {
    let opened_after_signatures = directory_chain_signatures(opened_chain)?;
    let verified_chain = open_workspace_directory_chain_no_follow(workspace, relative_path)?;
    let verified_signatures = directory_chain_signatures(&verified_chain)?;

    if opened_signatures.len() != opened_after_signatures.len()
        || opened_signatures.len() != verified_signatures.len()
        || opened_signatures != opened_after_signatures
        || opened_signatures != verified_signatures
    {
        return Err(WorkspaceDirectoryReadError::Changed(
            relative_path.to_owned(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn map_directory_open_error(error: rustix::io::Errno, path: &Path) -> WorkspaceDirectoryReadError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        WorkspaceDirectoryReadError::UnsafePath(path.to_owned())
    } else {
        WorkspaceDirectoryReadError::Io(io::Error::from(error))
    }
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

fn map_workspace_file_read_workspace_error(
    error: WorkspaceMetadataError,
) -> WorkspaceFileReadError {
    match error {
        WorkspaceMetadataError::InvalidPath(path) | WorkspaceMetadataError::UnsafePath(path) => {
            WorkspaceFileReadError::UnsafePath(path)
        }
        WorkspaceMetadataError::IdentityChanged(path) => {
            WorkspaceFileReadError::Workspace(WorkspaceMetadataError::IdentityChanged(path))
        }
        WorkspaceMetadataError::Io(error) => WorkspaceFileReadError::Io(error),
        WorkspaceMetadataError::UnsupportedPlatform => {
            WorkspaceFileReadError::Workspace(WorkspaceMetadataError::UnsupportedPlatform)
        }
    }
}

fn map_workspace_file_read_metadata_error(
    error: WorkspaceFileMetadataError,
) -> WorkspaceFileReadError {
    match error {
        WorkspaceFileMetadataError::Workspace(error) => {
            map_workspace_file_read_workspace_error(error)
        }
        WorkspaceFileMetadataError::InvalidRelativePath(path) => {
            WorkspaceFileReadError::InvalidRelativePath(path)
        }
        WorkspaceFileMetadataError::UnsafePath(path) => WorkspaceFileReadError::UnsafePath(path),
        WorkspaceFileMetadataError::NotRegularFile(path) => {
            WorkspaceFileReadError::NotRegularFile(path)
        }
        WorkspaceFileMetadataError::SensitivePath(path) => {
            WorkspaceFileReadError::SensitivePath(path)
        }
        WorkspaceFileMetadataError::FileTooLarge(path) => {
            WorkspaceFileReadError::FileTooLarge(path)
        }
        WorkspaceFileMetadataError::Changed(path) => WorkspaceFileReadError::Changed(path),
        WorkspaceFileMetadataError::Io(error) => WorkspaceFileReadError::Io(error),
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
    use super::{
        WorkspaceDirectoryEntryKind, WorkspaceDirectoryReadError, WorkspaceFileMetadataError,
        WorkspaceFileReadError, WorkspaceMetadata, WorkspaceMetadataError,
    };
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
    fn should_read_directory_entries_without_following_links_or_returning_sensitive_names() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let external_file = directory.0.join("external.txt");
        let external_directory = directory.0.join("external-directory");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::create_dir(&external_directory).unwrap();
        fs::write(workspace_path.join("visible.txt"), b"visible").unwrap();
        fs::write(workspace_path.join(".env.local"), b"synthetic secret").unwrap();
        fs::write(workspace_path.join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(&external_file, b"outside").unwrap();
        symlink(&external_file, workspace_path.join("linked.txt")).unwrap();
        symlink(&external_directory, workspace_path.join("linked-directory")).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let listing = workspace
            .read_directory_with_limit("", &workspace.directory_identity(), 10)
            .unwrap();
        let visible = listing
            .entries
            .iter()
            .find(|entry| entry.name == "visible.txt")
            .unwrap();
        let source_directory = listing
            .entries
            .iter()
            .find(|entry| entry.name == "src")
            .unwrap();
        let linked = listing
            .entries
            .iter()
            .find(|entry| entry.name == "linked.txt")
            .unwrap();

        assert!(!listing.truncated);
        assert_eq!(visible.kind, WorkspaceDirectoryEntryKind::RegularFile);
        assert_eq!(
            source_directory.kind,
            WorkspaceDirectoryEntryKind::Directory
        );
        assert_eq!(linked.kind, WorkspaceDirectoryEntryKind::Symlink);
        assert_eq!(listing.entries_seen, 5);
        assert_eq!(listing.sensitive_entries_skipped, 1);
        assert!(
            !listing
                .entries
                .iter()
                .any(|entry| entry.name == ".env.local")
        );
        assert!(matches!(
            workspace.read_directory_with_limit(
                "linked-directory",
                &workspace.directory_identity(),
                10
            ),
            Err(WorkspaceDirectoryReadError::UnsafePath(_))
        ));
        assert!(matches!(
            workspace.read_directory_with_limit("../outside", &workspace.directory_identity(), 10),
            Err(WorkspaceDirectoryReadError::InvalidRelativePath(_))
        ));

        let nested = workspace
            .read_directory_with_limit("src", source_directory.identity(), 10)
            .unwrap();
        assert_eq!(nested.entries.len(), 1);
        assert_eq!(nested.entries[0].name, "main.rs");
        assert_eq!(
            nested.entries[0].kind,
            WorkspaceDirectoryEntryKind::RegularFile
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_replaced_directories_and_bound_directory_entry_results() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(workspace_path.join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(workspace_path.join("one.txt"), b"one").unwrap();
        fs::write(workspace_path.join("two.txt"), b"two").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        let root_listing = workspace
            .read_directory_with_limit("", &workspace.directory_identity(), 10)
            .unwrap();
        let source_directory = root_listing
            .entries
            .iter()
            .find(|entry| entry.name == "src")
            .unwrap();
        let original_identity = source_directory.identity().clone();

        let limited = workspace
            .read_directory_with_limit("", &workspace.directory_identity(), 1)
            .unwrap();
        assert!(limited.truncated);
        assert!(limited.entries.len() <= 1);

        let empty_limit = workspace
            .read_directory_with_limit("", &workspace.directory_identity(), 0)
            .unwrap();
        assert!(empty_limit.truncated);
        assert!(empty_limit.entries.is_empty());

        fs::rename(
            workspace_path.join("src"),
            workspace_path.join("src-original"),
        )
        .unwrap();
        fs::create_dir(workspace_path.join("src")).unwrap();
        assert!(matches!(
            workspace.read_directory_with_limit("src", &original_identity, 10),
            Err(WorkspaceDirectoryReadError::Changed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_detect_directory_replacement_between_open_and_final_verification() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::write(workspace_path.join("src/main.rs"), b"fn main() {}\n").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let relative_path = PathBuf::from("src");
        let opened_chain =
            super::open_workspace_directory_chain_no_follow(&workspace, &relative_path).unwrap();
        let opened_signatures = super::directory_chain_signatures(&opened_chain).unwrap();

        fs::rename(
            workspace_path.join("src"),
            workspace_path.join("src-original"),
        )
        .unwrap();
        fs::create_dir(workspace_path.join("src")).unwrap();

        assert!(matches!(
            super::verify_workspace_directory_chain(
                &workspace,
                &relative_path,
                &opened_chain,
                &opened_signatures
            ),
            Err(WorkspaceDirectoryReadError::Changed(_))
        ));
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
    fn should_read_bounded_workspace_files_and_refuse_sensitive_symlinked_or_oversized_paths() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        let external = directory.0.join("external.txt");
        let external_directory = directory.0.join("external-directory");
        fs::create_dir(&workspace).unwrap();
        fs::write(&external, b"outside the workspace").unwrap();
        fs::create_dir(&external_directory).unwrap();
        fs::write(
            external_directory.join("file.txt"),
            b"outside through a directory link",
        )
        .unwrap();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(workspace.join(".env"), b"placeholder\n").unwrap();
        fs::write(workspace.join(".env.example"), b"TOKEN=placeholder\n").unwrap();
        fs::write(workspace.join(".npmrc"), b"//registry/:_authToken=value\n").unwrap();
        fs::write(workspace.join(".pypirc"), b"password=value\n").unwrap();
        fs::write(workspace.join(".netrc"), b"password value\n").unwrap();
        fs::write(workspace.join("kubeconfig"), b"token: value\n").unwrap();
        fs::write(workspace.join("terraform.tfstate"), b"state").unwrap();
        fs::write(workspace.join("id_rsa"), b"private key").unwrap();
        fs::write(workspace.join("id_ed25519"), b"private key").unwrap();
        fs::write(workspace.join("service.pem"), b"private key").unwrap();
        fs::write(workspace.join("service.key"), b"private key").unwrap();
        fs::create_dir(workspace.join("directory")).unwrap();
        fs::create_dir_all(workspace.join(".aws")).unwrap();
        fs::write(workspace.join(".aws/credentials"), b"credentials").unwrap();
        fs::write(
            workspace.join("large.txt"),
            vec![b'x'; super::MAX_WORKSPACE_FILE_READ_BYTES + 1],
        )
        .unwrap();
        fs::write(
            workspace.join("at-limit.txt"),
            vec![b'x'; super::MAX_WORKSPACE_FILE_READ_BYTES],
        )
        .unwrap();
        symlink(&external, workspace.join("linked.txt")).unwrap();
        symlink(&external_directory, workspace.join("linked-directory")).unwrap();
        let workspace_metadata = WorkspaceMetadata::capture(&workspace).unwrap();

        assert_eq!(
            workspace_metadata.read_file("src/main.rs").unwrap(),
            b"fn main() {}\n"
        );
        assert!(matches!(
            workspace_metadata.read_file("../external.txt"),
            Err(WorkspaceFileReadError::InvalidRelativePath(_))
        ));
        assert!(matches!(
            workspace_metadata.read_file("linked.txt"),
            Err(WorkspaceFileReadError::UnsafePath(_))
        ));
        assert!(matches!(
            workspace_metadata.read_file("linked-directory/file.txt"),
            Err(WorkspaceFileReadError::UnsafePath(_))
        ));
        assert!(matches!(
            workspace_metadata.read_file("directory"),
            Err(WorkspaceFileReadError::NotRegularFile(_))
        ));
        assert!(matches!(
            workspace_metadata.read_file("large.txt"),
            Err(WorkspaceFileReadError::FileTooLarge(_))
        ));
        assert_eq!(
            workspace_metadata.read_file("at-limit.txt").unwrap().len(),
            super::MAX_WORKSPACE_FILE_READ_BYTES
        );
        assert_eq!(
            workspace_metadata
                .read_file_with_limit("src/main.rs", 13)
                .unwrap(),
            b"fn main() {}\n"
        );
        assert!(matches!(
            workspace_metadata.read_file_with_limit("at-limit.txt", 12),
            Err(WorkspaceFileReadError::FileTooLarge(_))
        ));
        assert_eq!(
            workspace_metadata
                .read_file_with_limit("at-limit.txt", super::MAX_WORKSPACE_FILE_READ_BYTES + 1)
                .unwrap()
                .len(),
            super::MAX_WORKSPACE_FILE_READ_BYTES
        );

        for sensitive_path in [
            ".env",
            ".env.example",
            ".npmrc",
            ".pypirc",
            ".netrc",
            "kubeconfig",
            "terraform.tfstate",
            "id_rsa",
            "id_ed25519",
            "service.pem",
            "service.key",
            ".aws/credentials",
        ] {
            assert!(matches!(
                workspace_metadata.read_file(sensitive_path),
                Err(WorkspaceFileReadError::SensitivePath(_))
            ));
        }
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
