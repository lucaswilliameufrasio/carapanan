use super::{
    MAX_WORKSPACE_FILE_READ_BYTES, WorkspaceDirectoryReadError, WorkspaceFileMetadata,
    WorkspaceFileMetadataError, WorkspaceFileReadError, WorkspaceMetadata, WorkspaceMetadataError,
    map_workspace_file_read_metadata_error, open_workspace_directory_chain_no_follow,
    validate_relative_path,
};
use std::{
    error::Error,
    fmt, io,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

pub const MAX_WORKSPACE_FILE_WRITE_BYTES: usize = MAX_WORKSPACE_FILE_READ_BYTES;
const MAX_EXTENDED_ATTRIBUTE_LIST_BYTES: usize = 64 * 1024;
const MAX_EXTENDED_ATTRIBUTE_VALUE_BYTES: usize = 64 * 1024;
const MAX_EXTENDED_ATTRIBUTE_TOTAL_BYTES: usize = 1024 * 1024;
const TEMPORARY_FILE_ATTEMPTS: usize = 16;

/// An ephemeral, in-memory snapshot of one explicitly selected workspace file.
///
/// Contents and extended attributes are deliberately omitted from `Debug` and serialization.
pub struct WorkspaceFileSnapshot {
    workspace_device: u64,
    workspace_inode: u64,
    relative_path: PathBuf,
    metadata: WorkspaceFileMetadata,
    signature: FileSignature,
    contents: Vec<u8>,
    extended_attributes: Vec<ExtendedAttribute>,
}

impl WorkspaceFileSnapshot {
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    pub fn metadata(&self) -> &WorkspaceFileMetadata {
        &self.metadata
    }
}

#[derive(Debug)]
pub enum WorkspaceFileWriteError {
    Workspace(WorkspaceMetadataError),
    InvalidRelativePath(PathBuf),
    UnsafePath(PathBuf),
    SensitivePath(PathBuf),
    FileTooLarge(PathBuf),
    MetadataTooLarge(PathBuf),
    MetadataNotPreservable(PathBuf),
    SnapshotFromDifferentWorkspace,
    Conflict(PathBuf),
    Changed(PathBuf),
    Io(io::Error),
    CommitOutcomeUncertain(PathBuf),
}

impl fmt::Display for WorkspaceFileWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(formatter, "workspace validation failed: {error}"),
            Self::InvalidRelativePath(path) => write!(
                formatter,
                "workspace write path must be relative and stay within the workspace: {}",
                path.display()
            ),
            Self::UnsafePath(path) => write!(
                formatter,
                "refusing workspace write through an unsafe path: {}",
                path.display()
            ),
            Self::SensitivePath(path) => write!(
                formatter,
                "refusing to write a sensitive workspace path: {}",
                path.display()
            ),
            Self::FileTooLarge(path) => write!(
                formatter,
                "workspace write source or replacement exceeds 256 KiB: {}",
                path.display()
            ),
            Self::MetadataTooLarge(path) => write!(
                formatter,
                "workspace file metadata exceeds the snapshot limit: {}",
                path.display()
            ),
            Self::MetadataNotPreservable(path) => write!(
                formatter,
                "workspace file metadata cannot be preserved safely: {}",
                path.display()
            ),
            Self::SnapshotFromDifferentWorkspace => {
                formatter.write_str("workspace file snapshot belongs to a different workspace")
            }
            Self::Conflict(path) => write!(
                formatter,
                "workspace file changed since its snapshot: {}",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "workspace path changed while preparing the write: {}",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "workspace file replacement failed: {error}"),
            Self::CommitOutcomeUncertain(path) => write!(
                formatter,
                "workspace file replacement may have committed; inspect before retrying: {}",
                path.display()
            ),
        }
    }
}

impl Error for WorkspaceFileWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl WorkspaceMetadata {
    /// Read one bounded file and capture the bytes and metadata through the same open descriptor.
    ///
    /// The returned value is local, ephemeral data. It is not suitable for persistence or logging.
    pub fn read_file_snapshot(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<WorkspaceFileSnapshot, WorkspaceFileReadError> {
        let relative_path = relative_path.as_ref();
        self.verify_current()
            .map_err(WorkspaceFileReadError::Workspace)?;
        validate_relative_path(relative_path).map_err(map_workspace_file_read_metadata_error)?;
        if self.is_sensitive_relative_path(relative_path) {
            return Err(WorkspaceFileReadError::SensitivePath(
                relative_path.to_owned(),
            ));
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            read_file_snapshot_unix(self, relative_path)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = relative_path;
            Err(WorkspaceFileReadError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }

    /// Atomically replace the existing file represented by `snapshot` if it is still unchanged.
    ///
    /// This is an isolated filesystem primitive, not an approval or a lock against external
    /// writers. A concurrent mutation after the last comparison and before rename remains possible.
    pub fn replace_file_atomically(
        &self,
        snapshot: &WorkspaceFileSnapshot,
        replacement: &[u8],
    ) -> Result<(), WorkspaceFileWriteError> {
        self.verify_current()
            .map_err(WorkspaceFileWriteError::Workspace)?;
        if self.device != snapshot.workspace_device || self.inode != snapshot.workspace_inode {
            return Err(WorkspaceFileWriteError::SnapshotFromDifferentWorkspace);
        }
        let relative_path = &snapshot.relative_path;
        validate_relative_path(relative_path)
            .map_err(|_| WorkspaceFileWriteError::InvalidRelativePath(relative_path.to_owned()))?;
        if self.is_sensitive_relative_path(relative_path) {
            return Err(WorkspaceFileWriteError::SensitivePath(
                relative_path.to_owned(),
            ));
        }
        if snapshot.contents.len() > MAX_WORKSPACE_FILE_WRITE_BYTES
            || replacement.len() > MAX_WORKSPACE_FILE_WRITE_BYTES
        {
            return Err(WorkspaceFileWriteError::FileTooLarge(
                relative_path.to_owned(),
            ));
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            replace_file_atomically_unix(self, snapshot, replacement)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (snapshot, replacement);
            Err(WorkspaceFileWriteError::Workspace(
                WorkspaceMetadataError::UnsupportedPlatform,
            ))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_file_snapshot_unix(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
) -> Result<WorkspaceFileSnapshot, WorkspaceFileReadError> {
    use std::os::unix::fs::MetadataExt;

    let mut file = workspace
        .open_snapshot_file(relative_path)
        .map_err(map_workspace_file_read_metadata_error)?;
    let initial_metadata = file.metadata().map_err(WorkspaceFileReadError::Io)?;
    if !initial_metadata.file_type().is_file() {
        return Err(WorkspaceFileReadError::NotRegularFile(
            relative_path.to_owned(),
        ));
    }
    if initial_metadata.dev() != workspace.device {
        return Err(WorkspaceFileReadError::UnsafePath(relative_path.to_owned()));
    }
    if initial_metadata.len() > MAX_WORKSPACE_FILE_READ_BYTES as u64 {
        return Err(WorkspaceFileReadError::FileTooLarge(
            relative_path.to_owned(),
        ));
    }

    let initial_signature = FileSignature::from_metadata(&initial_metadata);
    let contents = read_bounded_contents(&mut file).map_err(|error| match error {
        SnapshotIoError::FileTooLarge => {
            WorkspaceFileReadError::FileTooLarge(relative_path.to_owned())
        }
        SnapshotIoError::Io(error) => WorkspaceFileReadError::Io(error),
    })?;
    let attributes_before = read_extended_attributes(&file).map_err(|error| match error {
        ExtendedAttributeError::TooLarge => {
            WorkspaceFileReadError::MetadataTooLarge(relative_path.to_owned())
        }
        ExtendedAttributeError::Io(error) => WorkspaceFileReadError::Io(error),
    })?;
    let final_metadata = file.metadata().map_err(WorkspaceFileReadError::Io)?;
    let attributes_after = read_extended_attributes(&file).map_err(|error| match error {
        ExtendedAttributeError::TooLarge => {
            WorkspaceFileReadError::MetadataTooLarge(relative_path.to_owned())
        }
        ExtendedAttributeError::Io(error) => WorkspaceFileReadError::Io(error),
    })?;
    if initial_signature != FileSignature::from_metadata(&final_metadata)
        || attributes_before != attributes_after
        || contents.len() as u64 != final_metadata.len()
    {
        return Err(WorkspaceFileReadError::Changed(relative_path.to_owned()));
    }

    Ok(WorkspaceFileSnapshot {
        workspace_device: workspace.device,
        workspace_inode: workspace.inode,
        relative_path: relative_path.to_owned(),
        metadata: WorkspaceFileMetadata::from_stat(relative_path, &final_metadata, None),
        signature: FileSignature::from_metadata(&final_metadata),
        contents,
        extended_attributes: attributes_after,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn replace_file_atomically_unix(
    workspace: &WorkspaceMetadata,
    snapshot: &WorkspaceFileSnapshot,
    replacement: &[u8],
) -> Result<(), WorkspaceFileWriteError> {
    use rustix::{fd::AsFd, fs::renameat};
    use std::os::unix::fs::MetadataExt;

    let relative_path = &snapshot.relative_path;
    if !snapshot.extended_attributes.is_empty() {
        return Err(WorkspaceFileWriteError::MetadataNotPreservable(
            relative_path.to_owned(),
        ));
    }
    let file_name = relative_path
        .file_name()
        .ok_or_else(|| WorkspaceFileWriteError::InvalidRelativePath(relative_path.to_owned()))?;
    let parent_path = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let directory_chain = open_workspace_directory_chain_no_follow(workspace, parent_path)
        .map_err(map_directory_read_error)?;
    verify_directory_chain_filesystem(workspace, relative_path, &directory_chain)?;
    let parent = directory_chain
        .last()
        .ok_or_else(|| WorkspaceFileWriteError::Changed(relative_path.to_owned()))?;

    verify_snapshot_still_current(parent, file_name, snapshot)?;
    verify_directory_chain_identity(workspace, parent_path, &directory_chain, relative_path)?;

    let (temporary_name, mut temporary_file) =
        create_temporary_file(parent).map_err(WorkspaceFileWriteError::Io)?;
    let mut temporary_entry = TemporaryEntry::new(parent, temporary_name.clone());
    let original_mode = snapshot.signature.mode;
    if original_mode & 0o7000 != 0 || snapshot.signature.link_count != 1 {
        return Err(WorkspaceFileWriteError::MetadataNotPreservable(
            relative_path.to_owned(),
        ));
    }

    let temporary_metadata = temporary_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    if temporary_metadata.uid() != snapshot.signature.owner_uid
        || temporary_metadata.gid() != snapshot.signature.group_gid
    {
        return Err(WorkspaceFileWriteError::MetadataNotPreservable(
            relative_path.to_owned(),
        ));
    }

    rustix::fs::fchmod(
        temporary_file.as_fd(),
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| WorkspaceFileWriteError::Io(io::Error::from(error)))?;
    temporary_file
        .write_all(replacement)
        .map_err(WorkspaceFileWriteError::Io)?;

    let temporary_metadata = temporary_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    if temporary_metadata.uid() != snapshot.signature.owner_uid
        || temporary_metadata.gid() != snapshot.signature.group_gid
        || temporary_metadata.mode() & 0o777 != 0o600
        || !read_extended_attributes(&temporary_file)
            .map_err(|error| map_extended_attribute_write_error(error, relative_path))?
            .is_empty()
    {
        return Err(WorkspaceFileWriteError::MetadataNotPreservable(
            relative_path.to_owned(),
        ));
    }
    temporary_file
        .sync_all()
        .map_err(WorkspaceFileWriteError::Io)?;

    verify_snapshot_still_current(parent, file_name, snapshot)?;
    verify_replacement_visible(
        parent,
        &temporary_name,
        &temporary_file,
        replacement,
        snapshot,
    )
    .map_err(|_| WorkspaceFileWriteError::Changed(relative_path.to_owned()))?;
    verify_directory_chain_identity(workspace, parent_path, &directory_chain, relative_path)?;
    verify_snapshot_still_current(parent, file_name, snapshot)?;

    renameat(parent.as_fd(), &temporary_name, parent.as_fd(), file_name)
        .map_err(|error| map_rename_error(error, relative_path))?;
    temporary_entry.mark_committed();

    if rustix::fs::fchmod(
        temporary_file.as_fd(),
        rustix::fs::Mode::from_bits_truncate(original_mode & 0o777),
    )
    .is_err()
        || temporary_file.sync_all().is_err()
        || parent.sync_all().is_err()
        || verify_directory_chain_identity(workspace, parent_path, &directory_chain, relative_path)
            .is_err()
        || verify_replacement_visible(parent, file_name, &temporary_file, replacement, snapshot)
            .is_err()
    {
        return Err(WorkspaceFileWriteError::CommitOutcomeUncertain(
            relative_path.to_owned(),
        ));
    }

    Ok(())
}

impl WorkspaceMetadata {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn open_snapshot_file(
        &self,
        relative_path: &Path,
    ) -> Result<std::fs::File, WorkspaceFileMetadataError> {
        super::open_workspace_file_no_follow(self, relative_path)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_contents(file: &mut std::fs::File) -> Result<Vec<u8>, SnapshotIoError> {
    let mut contents = Vec::new();
    Read::by_ref(file)
        .take((MAX_WORKSPACE_FILE_READ_BYTES as u64).saturating_add(1))
        .read_to_end(&mut contents)
        .map_err(SnapshotIoError::Io)?;
    if contents.len() > MAX_WORKSPACE_FILE_READ_BYTES {
        return Err(SnapshotIoError::FileTooLarge);
    }
    Ok(contents)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_snapshot_still_current(
    parent: &std::fs::File,
    file_name: &std::ffi::OsStr,
    snapshot: &WorkspaceFileSnapshot,
) -> Result<(), WorkspaceFileWriteError> {
    use rustix::{fd::AsFd, fs::openat};
    use std::os::unix::fs::MetadataExt;

    let descriptor = openat(
        parent.as_fd(),
        file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| map_target_open_error(error, &snapshot.relative_path))?;
    let mut current_file = std::fs::File::from(descriptor);
    let initial_metadata = current_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    if !initial_metadata.file_type().is_file() {
        return Err(WorkspaceFileWriteError::Conflict(
            snapshot.relative_path.clone(),
        ));
    }
    if initial_metadata.dev() != snapshot.workspace_device {
        return Err(WorkspaceFileWriteError::UnsafePath(
            snapshot.relative_path.clone(),
        ));
    }
    if FileSignature::from_metadata(&initial_metadata) != snapshot.signature {
        return Err(WorkspaceFileWriteError::Conflict(
            snapshot.relative_path.clone(),
        ));
    }
    let contents = read_bounded_contents(&mut current_file).map_err(|error| match error {
        SnapshotIoError::FileTooLarge => {
            WorkspaceFileWriteError::FileTooLarge(snapshot.relative_path.clone())
        }
        SnapshotIoError::Io(error) => WorkspaceFileWriteError::Io(error),
    })?;
    let attributes_before = read_extended_attributes(&current_file)
        .map_err(|error| map_extended_attribute_write_error(error, &snapshot.relative_path))?;
    let final_metadata = current_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    let attributes_after = read_extended_attributes(&current_file)
        .map_err(|error| map_extended_attribute_write_error(error, &snapshot.relative_path))?;
    if FileSignature::from_metadata(&final_metadata) != snapshot.signature
        || attributes_before != attributes_after
        || attributes_after != snapshot.extended_attributes
        || contents != snapshot.contents
    {
        return Err(WorkspaceFileWriteError::Conflict(
            snapshot.relative_path.clone(),
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_replacement_visible(
    parent: &std::fs::File,
    file_name: &std::ffi::OsStr,
    temporary_file: &std::fs::File,
    replacement: &[u8],
    snapshot: &WorkspaceFileSnapshot,
) -> Result<(), WorkspaceFileWriteError> {
    use rustix::{fd::AsFd, fs::openat};
    use std::os::unix::fs::MetadataExt;

    let descriptor = openat(
        parent.as_fd(),
        file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| WorkspaceFileWriteError::Io(io::Error::from(error)))?;
    let mut visible_file = std::fs::File::from(descriptor);
    let visible_metadata = visible_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    let temporary_metadata = temporary_file
        .metadata()
        .map_err(WorkspaceFileWriteError::Io)?;
    if !visible_metadata.file_type().is_file()
        || visible_metadata.dev() != temporary_metadata.dev()
        || visible_metadata.ino() != temporary_metadata.ino()
    {
        return Err(WorkspaceFileWriteError::Conflict(
            snapshot.relative_path.clone(),
        ));
    }
    let visible_contents =
        read_bounded_contents(&mut visible_file).map_err(|error| match error {
            SnapshotIoError::FileTooLarge => {
                WorkspaceFileWriteError::CommitOutcomeUncertain(snapshot.relative_path.clone())
            }
            SnapshotIoError::Io(error) => WorkspaceFileWriteError::Io(error),
        })?;
    let visible_attributes = read_extended_attributes(&visible_file).map_err(|_| {
        WorkspaceFileWriteError::CommitOutcomeUncertain(snapshot.relative_path.clone())
    })?;
    if visible_contents != replacement
        || visible_attributes != snapshot.extended_attributes
        || visible_metadata.uid() != temporary_metadata.uid()
        || visible_metadata.gid() != temporary_metadata.gid()
        || visible_metadata.mode() & 0o777 != temporary_metadata.mode() & 0o777
    {
        return Err(WorkspaceFileWriteError::Conflict(
            snapshot.relative_path.clone(),
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_directory_chain_filesystem(
    workspace: &WorkspaceMetadata,
    relative_path: &Path,
    directories: &[std::fs::File],
) -> Result<(), WorkspaceFileWriteError> {
    use std::os::unix::fs::MetadataExt;

    for directory in directories {
        let metadata = directory.metadata().map_err(WorkspaceFileWriteError::Io)?;
        if !metadata.is_dir() || metadata.dev() != workspace.device {
            return Err(WorkspaceFileWriteError::UnsafePath(
                relative_path.to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_directory_chain_identity(
    workspace: &WorkspaceMetadata,
    parent_path: &Path,
    opened_chain: &[std::fs::File],
    relative_path: &Path,
) -> Result<(), WorkspaceFileWriteError> {
    use std::os::unix::fs::MetadataExt;

    let reopened_chain = open_workspace_directory_chain_no_follow(workspace, parent_path)
        .map_err(map_directory_read_error)?;
    if opened_chain.len() != reopened_chain.len() {
        return Err(WorkspaceFileWriteError::Changed(relative_path.to_owned()));
    }
    for (opened, reopened) in opened_chain.iter().zip(reopened_chain.iter()) {
        let opened_metadata = opened.metadata().map_err(WorkspaceFileWriteError::Io)?;
        let reopened_metadata = reopened.metadata().map_err(WorkspaceFileWriteError::Io)?;
        if !opened_metadata.is_dir()
            || !reopened_metadata.is_dir()
            || opened_metadata.dev() != reopened_metadata.dev()
            || opened_metadata.ino() != reopened_metadata.ino()
        {
            return Err(WorkspaceFileWriteError::Changed(relative_path.to_owned()));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_temporary_file(
    parent: &std::fs::File,
) -> Result<(std::ffi::OsString, std::fs::File), io::Error> {
    use rustix::{fd::AsFd, fs::openat};

    for _attempt in 0..TEMPORARY_FILE_ATTEMPTS {
        let name = temporary_name();
        let descriptor = match openat(
            parent.as_fd(),
            &name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::from_bits_truncate(0o600),
        ) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                let error = io::Error::from(error);
                if error.kind() == io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(error);
            }
        };
        return Ok((name, std::fs::File::from(descriptor)));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique workspace temporary file",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn temporary_name() -> std::ffi::OsString {
    static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
    std::ffi::OsString::from(format!(
        ".carapana-write-{}-{timestamp}-{sequence}.tmp",
        std::process::id()
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TemporaryEntry<'a> {
    parent: &'a std::fs::File,
    name: std::ffi::OsString,
    committed: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a> TemporaryEntry<'a> {
    fn new(parent: &'a std::fs::File, name: std::ffi::OsString) -> Self {
        Self {
            parent,
            name,
            committed: false,
        }
    }

    fn mark_committed(&mut self) {
        self.committed = true;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for TemporaryEntry<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let _ = rustix::fs::unlinkat(self.parent, &self.name, rustix::fs::AtFlags::empty());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_target_open_error(
    error: rustix::io::Errno,
    relative_path: &Path,
) -> WorkspaceFileWriteError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        return WorkspaceFileWriteError::UnsafePath(relative_path.to_owned());
    }
    let error = io::Error::from(error);
    if error.kind() == io::ErrorKind::NotFound {
        return WorkspaceFileWriteError::Conflict(relative_path.to_owned());
    }
    WorkspaceFileWriteError::Io(error)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_rename_error(error: rustix::io::Errno, relative_path: &Path) -> WorkspaceFileWriteError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        return WorkspaceFileWriteError::UnsafePath(relative_path.to_owned());
    }
    WorkspaceFileWriteError::Io(io::Error::from(error))
}

fn map_directory_read_error(error: WorkspaceDirectoryReadError) -> WorkspaceFileWriteError {
    match error {
        WorkspaceDirectoryReadError::Workspace(error) => WorkspaceFileWriteError::Workspace(error),
        WorkspaceDirectoryReadError::InvalidRelativePath(path) => {
            WorkspaceFileWriteError::InvalidRelativePath(path)
        }
        WorkspaceDirectoryReadError::UnsafePath(path)
        | WorkspaceDirectoryReadError::SensitivePath(path) => {
            WorkspaceFileWriteError::UnsafePath(path)
        }
        WorkspaceDirectoryReadError::NotDirectory(path)
        | WorkspaceDirectoryReadError::Changed(path) => WorkspaceFileWriteError::Changed(path),
        WorkspaceDirectoryReadError::Io(error) => WorkspaceFileWriteError::Io(error),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExtendedAttribute {
    name: Vec<u8>,
    value: Vec<u8>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
enum ExtendedAttributeError {
    TooLarge,
    Io(io::Error),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_extended_attributes(
    file: &std::fs::File,
) -> Result<Vec<ExtendedAttribute>, ExtendedAttributeError> {
    use rustix::{buffer::spare_capacity, fd::AsFd, fs::flistxattr};
    use std::os::unix::ffi::OsStrExt;

    let mut names = Vec::with_capacity(MAX_EXTENDED_ATTRIBUTE_LIST_BYTES);
    flistxattr(file.as_fd(), spare_capacity(&mut names))
        .map_err(|error| ExtendedAttributeError::Io(io::Error::from(error)))?;
    if names.len() > MAX_EXTENDED_ATTRIBUTE_TOTAL_BYTES {
        return Err(ExtendedAttributeError::TooLarge);
    }

    let mut total_bytes = names.len();
    let mut attributes = Vec::new();
    let mut offset = 0_usize;
    while offset < names.len() {
        let Some(name_length) = names[offset..].iter().position(|byte| *byte == 0) else {
            return Err(ExtendedAttributeError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem returned a malformed extended-attribute list",
            )));
        };
        if name_length == 0 {
            return Err(ExtendedAttributeError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem returned an empty extended-attribute name",
            )));
        }
        let name = names[offset..offset + name_length].to_vec();
        offset += name_length + 1;
        let remaining_bytes = MAX_EXTENDED_ATTRIBUTE_TOTAL_BYTES.saturating_sub(total_bytes);
        if remaining_bytes == 0 {
            return Err(ExtendedAttributeError::TooLarge);
        }
        let value_limit = remaining_bytes.min(MAX_EXTENDED_ATTRIBUTE_VALUE_BYTES);
        let mut value = Vec::with_capacity(value_limit);
        rustix::fs::fgetxattr(
            file.as_fd(),
            std::ffi::OsStr::from_bytes(&name),
            spare_capacity(&mut value),
        )
        .map_err(|error| ExtendedAttributeError::Io(io::Error::from(error)))?;
        total_bytes = total_bytes
            .checked_add(name.len())
            .and_then(|count| count.checked_add(value.len()))
            .ok_or(ExtendedAttributeError::TooLarge)?;
        if total_bytes > MAX_EXTENDED_ATTRIBUTE_TOTAL_BYTES {
            return Err(ExtendedAttributeError::TooLarge);
        }
        attributes.push(ExtendedAttribute { name, value });
    }
    attributes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(attributes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_extended_attribute_write_error(
    error: ExtendedAttributeError,
    relative_path: &Path,
) -> WorkspaceFileWriteError {
    match error {
        ExtendedAttributeError::TooLarge => {
            WorkspaceFileWriteError::MetadataTooLarge(relative_path.to_owned())
        }
        ExtendedAttributeError::Io(error) => WorkspaceFileWriteError::Io(error),
    }
}

enum SnapshotIoError {
    FileTooLarge,
    Io(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileSignature {
    device: u64,
    inode: u64,
    mode: u32,
    owner_uid: u32,
    group_gid: u32,
    link_count: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FileSignature {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;

        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            owner_uid: metadata.uid(),
            group_gid: metadata.gid(),
            link_count: metadata.nlink(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WORKSPACE_FILE_WRITE_BYTES, WorkspaceFileWriteError};
    use crate::WorkspaceMetadata;
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
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-workspace-write-{}-{timestamp}-{sequence}",
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
    fn should_replace_an_existing_file_atomically_and_preserve_basic_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        let file_path = workspace_path.join("src/main.rs");
        fs::write(&file_path, b"fn old() {}\n").unwrap();
        fs::set_permissions(&file_path, fs::Permissions::from_mode(0o640)).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("src/main.rs").unwrap();
        assert_eq!(snapshot.contents(), b"fn old() {}\n");

        workspace
            .replace_file_atomically(&snapshot, b"fn new() {}\n")
            .unwrap();

        assert_eq!(fs::read(&file_path).unwrap(), b"fn new() {}\n");
        assert_eq!(fs::metadata(&file_path).unwrap().mode() & 0o777, 0o640);
        assert_eq!(
            fs::read_dir(file_path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_not_overwrite_a_file_changed_since_its_snapshot() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        let file_path = workspace_path.join("notes.txt");
        fs::write(&file_path, b"original").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("notes.txt").unwrap();
        fs::write(&file_path, b"operator edit").unwrap();

        assert!(matches!(
            workspace.replace_file_atomically(&snapshot, b"agent edit"),
            Err(WorkspaceFileWriteError::Conflict(_))
        ));
        assert_eq!(fs::read(file_path).unwrap(), b"operator edit");
        assert_eq!(fs::read_dir(&workspace_path).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn should_not_create_a_file_when_the_snapshot_target_is_already_missing() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        let file_path = workspace_path.join("notes.txt");
        fs::write(&file_path, b"original").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("notes.txt").unwrap();
        fs::remove_file(&file_path).unwrap();

        assert!(matches!(
            workspace.replace_file_atomically(&snapshot, b"replacement"),
            Err(WorkspaceFileWriteError::Conflict(_))
        ));
        assert!(!file_path.exists());
        assert_eq!(fs::read_dir(&workspace_path).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_sensitive_paths_symlinks_and_paths_outside_the_workspace() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join(".env.example"), b"placeholder").unwrap();
        let outside_file = directory.0.join("outside.txt");
        fs::write(&outside_file, b"outside").unwrap();
        symlink(&outside_file, workspace_path.join("linked.txt")).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();

        assert!(matches!(
            workspace.read_file_snapshot(".env.example"),
            Err(crate::WorkspaceFileReadError::SensitivePath(_))
        ));
        assert!(matches!(
            workspace.read_file_snapshot("linked.txt"),
            Err(crate::WorkspaceFileReadError::UnsafePath(_))
        ));
        assert!(matches!(
            workspace.read_file_snapshot("../outside.txt"),
            Err(crate::WorkspaceFileReadError::InvalidRelativePath(_))
        ));
        assert_eq!(fs::read(outside_file).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_symlink_replacements_and_parent_directory_swaps() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        let outside_directory = directory.0.join("outside");
        fs::create_dir_all(workspace_path.join("src")).unwrap();
        fs::create_dir(&outside_directory).unwrap();
        let source_path = workspace_path.join("src/main.rs");
        let outside_file = directory.0.join("outside.txt");
        fs::write(&source_path, b"original").unwrap();
        fs::write(&outside_file, b"outside").unwrap();
        fs::write(outside_directory.join("main.rs"), b"outside directory").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("src/main.rs").unwrap();

        fs::remove_file(&source_path).unwrap();
        symlink(&outside_file, &source_path).unwrap();
        assert!(matches!(
            workspace.replace_file_atomically(&snapshot, b"replacement"),
            Err(WorkspaceFileWriteError::UnsafePath(_))
        ));
        assert_eq!(fs::read(&outside_file).unwrap(), b"outside");

        fs::remove_file(&source_path).unwrap();
        fs::write(&source_path, b"original").unwrap();
        let second_snapshot = workspace.read_file_snapshot("src/main.rs").unwrap();
        fs::rename(
            workspace_path.join("src"),
            workspace_path.join("src-original"),
        )
        .unwrap();
        symlink(&outside_directory, workspace_path.join("src")).unwrap();
        assert!(matches!(
            workspace.replace_file_atomically(&second_snapshot, b"replacement"),
            Err(WorkspaceFileWriteError::UnsafePath(_))
        ));
        assert_eq!(
            fs::read(outside_directory.join("main.rs")).unwrap(),
            b"outside directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_snapshot_from_another_workspace_and_oversized_replacements() {
        let directory = TestDirectory::new();
        let first_path = directory.0.join("first");
        let second_path = directory.0.join("second");
        fs::create_dir(&first_path).unwrap();
        fs::create_dir(&second_path).unwrap();
        fs::write(first_path.join("file.txt"), b"same bytes").unwrap();
        fs::write(second_path.join("file.txt"), b"same bytes").unwrap();
        let first = WorkspaceMetadata::capture(&first_path).unwrap();
        let second = WorkspaceMetadata::capture(&second_path).unwrap();
        let snapshot = first.read_file_snapshot("file.txt").unwrap();

        assert!(matches!(
            second.replace_file_atomically(&snapshot, b"replacement"),
            Err(WorkspaceFileWriteError::SnapshotFromDifferentWorkspace)
        ));
        let oversized = vec![b'x'; MAX_WORKSPACE_FILE_WRITE_BYTES + 1];
        assert!(matches!(
            first.replace_file_atomically(&snapshot, &oversized),
            Err(WorkspaceFileWriteError::FileTooLarge(_))
        ));
        assert_eq!(
            fs::read(second_path.join("file.txt")).unwrap(),
            b"same bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_files_with_multiple_hard_links() {
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        let file_path = workspace_path.join("file.txt");
        let second_link = workspace_path.join("file-alias.txt");
        fs::write(&file_path, b"shared inode").unwrap();
        fs::hard_link(&file_path, &second_link).unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("file.txt").unwrap();

        assert!(matches!(
            workspace.replace_file_atomically(&snapshot, b"replacement"),
            Err(WorkspaceFileWriteError::MetadataNotPreservable(_))
        ));
        assert_eq!(fs::read(&file_path).unwrap(), b"shared inode");
        assert_eq!(fs::read(&second_link).unwrap(), b"shared inode");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn should_refuse_extended_attributes_without_changing_the_original_file() {
        use rustix::{fd::AsFd, fs::fsetxattr};
        use std::{fs::OpenOptions, os::unix::ffi::OsStrExt};

        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        let file_path = workspace_path.join("file.txt");
        fs::write(&file_path, b"before").unwrap();
        let file = OpenOptions::new().read(true).open(&file_path).unwrap();
        fsetxattr(
            file.as_fd(),
            std::ffi::OsStr::from_bytes(b"user.carapana-test"),
            b"synthetic attribute",
            rustix::fs::XattrFlags::empty(),
        )
        .unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        let snapshot = workspace.read_file_snapshot("file.txt").unwrap();

        assert!(matches!(
            workspace.replace_file_atomically(&snapshot, b"after"),
            Err(WorkspaceFileWriteError::MetadataNotPreservable(_))
        ));
        assert_eq!(fs::read(&file_path).unwrap(), b"before");

        let replaced = OpenOptions::new().read(true).open(file_path).unwrap();
        let mut value = Vec::with_capacity(b"synthetic attribute".len());
        rustix::fs::fgetxattr(
            replaced.as_fd(),
            std::ffi::OsStr::from_bytes(b"user.carapana-test"),
            rustix::buffer::spare_capacity(&mut value),
        )
        .unwrap();
        assert_eq!(value, b"synthetic attribute");
    }
}
