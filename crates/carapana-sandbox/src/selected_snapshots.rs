//! Private, ephemeral staging for explicitly selected files. No execution, sharing, or copy-back.
//!
//! Selection is caller input, not authenticated authorization or proof that content is secret-free.
//! A future launcher must enforce its own gates, pin identities, and mount this staging read-only.

use carapana_storage::{
    ArtifactStore, MAX_WORKSPACE_FILE_READ_BYTES, StagingReservation, WorkspaceDirectoryEntryKind,
    WorkspaceMetadata,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

const MAX_SELECTED_FILES: usize = 256;
const MAX_SELECTED_BYTES: usize = 16 * 1024 * 1024;
const MAX_SELECTED_PATH_BYTES: usize = 4096;
const MAX_SELECTED_PATH_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotPreparationError {
    UnsupportedPlatform,
    InvalidPolicy,
    InvalidPath,
    DuplicatePath,
    TooManyFiles,
    ByteBudgetExceeded,
    SourceRejected,
    StagingUnavailable,
    StagingChanged,
    ReservationRejected,
    CleanupPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedSnapshotPolicy {
    max_files: usize,
    max_total_bytes: usize,
}

impl SelectedSnapshotPolicy {
    pub fn new(max_files: usize, max_total_bytes: usize) -> Result<Self, SnapshotPreparationError> {
        if max_files == 0
            || max_files > MAX_SELECTED_FILES
            || max_total_bytes == 0
            || max_total_bytes > MAX_SELECTED_BYTES
        {
            return Err(SnapshotPreparationError::InvalidPolicy);
        }
        Ok(Self {
            max_files,
            max_total_bytes,
        })
    }
}

/// Paths and source-derived payload metadata deliberately do not implement Debug or serialization.
/// Cleanup on drop is best-effort: crashes or filesystem failures can leave a private staging tree.
/// Use `ReservedSelectedSnapshotBundle` for orphan recovery and global quota accounting; this type alone is for
/// isolated synthetic probes and bounded local preparation, not a general launcher input.
///
/// ```compile_fail
/// fn log(bundle: &carapana_sandbox::selected_snapshots::SelectedSnapshotBundle) {
///     let _unsafe_log = format!("{bundle:?}");
/// }
/// ```
pub struct SelectedSnapshotBundle {
    root: PathBuf,
    identity: WorkspaceMetadata,
    files: Vec<ManifestEntry>,
    total_bytes: usize,
}

struct ManifestEntry {
    path: PathBuf,
    size: usize,
    sha256: [u8; 32],
}

/// Selected snapshots with a live reservation in the user's shared artifact/staging quota.
/// The future launcher must require this type, not an unaccounted synthetic bundle.
///
/// ```compile_fail
/// fn log(bundle: &carapana_sandbox::selected_snapshots::ReservedSelectedSnapshotBundle<'_>) {
///     let _unsafe_log = format!("{bundle:?}");
/// }
/// ```
pub struct ReservedSelectedSnapshotBundle<'store> {
    // Rust drops fields in declaration order: remove the bundle before releasing its quota lease.
    bundle: SelectedSnapshotBundle,
    reservation: StagingReservation<'store>,
}

impl<'store> ReservedSelectedSnapshotBundle<'store> {
    pub fn prepare(
        store: &'store mut ArtifactStore,
        workspace: &WorkspaceMetadata,
        paths: &[PathBuf],
        policy: SelectedSnapshotPolicy,
        now_ms: i64,
    ) -> Result<Self, SnapshotPreparationError> {
        #[cfg(unix)]
        {
            validate_selection(workspace, paths, policy)?;
            workspace
                .verify_current()
                .map_err(|_| SnapshotPreparationError::SourceRejected)?;
            let reservation = store
                .reserve_staging(policy.max_total_bytes as u64, now_ms)
                .map_err(|_| SnapshotPreparationError::ReservationRejected)?;
            let bundle = prepare_in_directory(
                workspace,
                paths,
                policy,
                reservation.directory_metadata().clone(),
            )?;
            Ok(Self {
                bundle,
                reservation,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (store, workspace, paths, policy, now_ms);
            Err(SnapshotPreparationError::UnsupportedPlatform)
        }
    }

    pub fn staging_root(&self) -> &Path {
        self.bundle.staging_root()
    }
    pub fn file_count(&self) -> usize {
        self.bundle.file_count()
    }
    pub fn total_bytes(&self) -> usize {
        self.bundle.total_bytes()
    }
    pub fn reserved_bytes(&self) -> u64 {
        self.reservation.reserved_bytes()
    }
    pub fn verify(&self) -> Result<(), SnapshotPreparationError> {
        self.bundle.verify()
    }

    pub fn release(self) -> Result<(), SnapshotPreparationError> {
        let Self {
            bundle,
            reservation,
        } = self;
        drop(bundle);
        reservation
            .release()
            .map_err(|_| SnapshotPreparationError::CleanupPending)
    }
}

impl SelectedSnapshotBundle {
    /// Copy only selected files using bounded source reads. Does not authenticate selection or inspect secrets.
    pub fn prepare(
        workspace: &WorkspaceMetadata,
        paths: &[PathBuf],
        policy: SelectedSnapshotPolicy,
    ) -> Result<Self, SnapshotPreparationError> {
        #[cfg(unix)]
        {
            prepare_unix(workspace, paths, policy)
        }
        #[cfg(not(unix))]
        {
            let _ = (workspace, paths, policy);
            Err(SnapshotPreparationError::UnsupportedPlatform)
        }
    }

    /// Internal mount source for the future launcher, not an authorized host path or display value.
    pub fn staging_root(&self) -> &Path {
        &self.root
    }
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }
    pub fn selected_paths(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.files.iter().map(|entry| entry.path.as_path())
    }

    /// Observe that selected staging bytes still match the manifest. Not a concurrent-write lock.
    pub fn verify(&self) -> Result<(), SnapshotPreparationError> {
        self.identity
            .verify_current()
            .map_err(|_| SnapshotPreparationError::StagingChanged)?;
        self.verify_inventory()?;
        for entry in &self.files {
            let bytes = self
                .identity
                .read_file_with_limit(&entry.path, entry.size)
                .map_err(|_| SnapshotPreparationError::StagingChanged)?;
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if bytes.len() != entry.size || digest != entry.sha256 {
                return Err(SnapshotPreparationError::StagingChanged);
            }
        }
        Ok(())
    }

    fn verify_inventory(&self) -> Result<(), SnapshotPreparationError> {
        let mut expected_files = HashSet::new();
        let mut expected_directories = HashSet::new();
        for entry in &self.files {
            expected_files.insert(entry.path.as_path());
            let mut parent = entry.path.parent();
            while let Some(path) = parent {
                if path.as_os_str().is_empty() {
                    break;
                }
                expected_directories.insert(path.to_owned());
                parent = path.parent();
            }
        }
        let entry_limit = expected_files.len() + expected_directories.len() + 1;
        let mut pending = vec![(PathBuf::new(), self.identity.directory_identity())];
        while let Some((path, identity)) = pending.pop() {
            let listing = self
                .identity
                .read_directory_with_limit(&path, &identity, entry_limit)
                .map_err(|_| SnapshotPreparationError::StagingChanged)?;
            if listing.truncated
                || listing.sensitive_entries_skipped > 0
                || listing.entries_seen != listing.entries.len()
            {
                return Err(SnapshotPreparationError::StagingChanged);
            }
            for entry in listing.entries {
                let child_path = path.join(&entry.name);
                match entry.kind {
                    WorkspaceDirectoryEntryKind::RegularFile
                        if expected_files.contains(child_path.as_path()) => {}
                    WorkspaceDirectoryEntryKind::Directory
                        if expected_directories.contains(&child_path) =>
                    {
                        pending.push((child_path, entry.identity().clone()));
                    }
                    _ => return Err(SnapshotPreparationError::StagingChanged),
                }
            }
        }
        Ok(())
    }
}

impl Drop for SelectedSnapshotBundle {
    fn drop(&mut self) {
        // Only the fresh private tree created by this operation; never the source workspace.
        if self.identity.verify_current().is_ok() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn validate_selection(
    workspace: &WorkspaceMetadata,
    paths: &[PathBuf],
    policy: SelectedSnapshotPolicy,
) -> Result<(), SnapshotPreparationError> {
    if paths.len() > policy.max_files {
        return Err(SnapshotPreparationError::TooManyFiles);
    }
    let mut selected = HashSet::new();
    for path in paths {
        if path.as_os_str().is_empty()
            || path.as_os_str().as_encoded_bytes().len() > MAX_SELECTED_PATH_BYTES
            || path.as_os_str().as_encoded_bytes().contains(&0)
            || path.components().count() > MAX_SELECTED_PATH_DEPTH
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(SnapshotPreparationError::InvalidPath);
        }
        if workspace.is_sensitive_relative_path(path) {
            return Err(SnapshotPreparationError::SourceRejected);
        }
        if !selected.insert(path) {
            return Err(SnapshotPreparationError::DuplicatePath);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn prepare_unix(
    workspace: &WorkspaceMetadata,
    paths: &[PathBuf],
    policy: SelectedSnapshotPolicy,
) -> Result<SelectedSnapshotBundle, SnapshotPreparationError> {
    validate_selection(workspace, paths, policy)?;
    workspace
        .verify_current()
        .map_err(|_| SnapshotPreparationError::SourceRejected)?;
    let root = create_private_root()?;
    let identity = match WorkspaceMetadata::capture(&root) {
        Ok(identity) => identity,
        Err(_) => {
            let _ = std::fs::remove_dir(&root);
            return Err(SnapshotPreparationError::StagingUnavailable);
        }
    };
    prepare_in_directory(workspace, paths, policy, identity)
}

#[cfg(unix)]
fn prepare_in_directory(
    workspace: &WorkspaceMetadata,
    paths: &[PathBuf],
    policy: SelectedSnapshotPolicy,
    identity: WorkspaceMetadata,
) -> Result<SelectedSnapshotBundle, SnapshotPreparationError> {
    use std::{
        fs,
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    identity
        .verify_current()
        .map_err(|_| SnapshotPreparationError::StagingChanged)?;
    let root = identity.path();
    let mut bundle = SelectedSnapshotBundle {
        root,
        identity,
        files: Vec::new(),
        total_bytes: 0,
    };
    for path in paths {
        let remaining = policy.max_total_bytes - bundle.total_bytes;
        let metadata = workspace
            .observe_file(path)
            .map_err(|_| SnapshotPreparationError::SourceRejected)?;
        if metadata.size_bytes() > remaining as u64 {
            return Err(SnapshotPreparationError::ByteBudgetExceeded);
        }
        let bytes = workspace
            .read_file_with_limit(path, remaining.min(MAX_WORKSPACE_FILE_READ_BYTES))
            .map_err(|_| SnapshotPreparationError::SourceRejected)?;
        let mut directory = bundle.root.clone();
        if let Some(parent) = path.parent() {
            for component in parent.components() {
                directory.push(component.as_os_str());
                match fs::DirBuilder::new().mode(0o700).create(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        if !fs::symlink_metadata(&directory)
                            .map_err(|_| SnapshotPreparationError::StagingUnavailable)?
                            .is_dir()
                        {
                            return Err(SnapshotPreparationError::StagingUnavailable);
                        }
                    }
                    Err(_) => return Err(SnapshotPreparationError::StagingUnavailable),
                }
            }
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(bundle.root.join(path))
            .map_err(|_| SnapshotPreparationError::StagingUnavailable)?;
        file.write_all(&bytes)
            .map_err(|_| SnapshotPreparationError::StagingUnavailable)?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        bundle.total_bytes += bytes.len();
        bundle.files.push(ManifestEntry {
            path: path.clone(),
            size: bytes.len(),
            sha256: digest,
        });
    }
    workspace
        .verify_current()
        .map_err(|_| SnapshotPreparationError::SourceRejected)?;
    bundle.verify()?;
    Ok(bundle)
}

#[cfg(unix)]
fn create_private_root() -> Result<PathBuf, SnapshotPreparationError> {
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let base = fs::canonicalize(std::env::temp_dir())
        .map_err(|_| SnapshotPreparationError::StagingUnavailable)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SnapshotPreparationError::StagingUnavailable)?
        .as_nanos();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = base.join(format!(
        "carapana-selected-{}-{nonce}-{id}",
        std::process::id()
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|_| SnapshotPreparationError::StagingUnavailable)?;
    Ok(path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    struct Workspace {
        root: PathBuf,
        metadata: WorkspaceMetadata,
    }
    impl Workspace {
        fn new() -> Self {
            let root = create_private_root().unwrap();
            fs::create_dir(root.join("src")).unwrap();
            fs::write(root.join("src/main.rs"), b"synthetic source\n").unwrap();
            fs::write(root.join("not-selected"), b"not copied").unwrap();
            fs::write(root.join(".env.example"), b"synthetic protected fixture").unwrap();
            let metadata = WorkspaceMetadata::capture(&root).unwrap();
            Self { root, metadata }
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn policy() -> SelectedSnapshotPolicy {
        SelectedSnapshotPolicy::new(8, 1024).unwrap()
    }

    #[test]
    fn should_copy_only_selected_files_into_a_private_ephemeral_tree() {
        let workspace = Workspace::new();
        let bundle = SelectedSnapshotBundle::prepare(
            &workspace.metadata,
            &[PathBuf::from("src/main.rs")],
            policy(),
        )
        .unwrap();
        assert_eq!(bundle.file_count(), 1);
        assert_eq!(bundle.total_bytes(), 17);
        assert_eq!(
            fs::read(bundle.staging_root().join("src/main.rs")).unwrap(),
            b"synthetic source\n"
        );
        assert!(!bundle.staging_root().join(".env.example").exists());
        assert!(!bundle.staging_root().join("not-selected").exists());
        assert_eq!(
            fs::metadata(bundle.staging_root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(bundle.staging_root().join("src/main.rs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let path = bundle.staging_root().to_owned();
        drop(bundle);
        assert!(!path.exists());
        assert_eq!(
            fs::read(workspace.root.join("src/main.rs")).unwrap(),
            b"synthetic source\n"
        );
    }

    #[test]
    fn should_not_observe_later_source_edits_in_the_staged_snapshot() {
        let workspace = Workspace::new();
        let bundle = SelectedSnapshotBundle::prepare(
            &workspace.metadata,
            &[PathBuf::from("src/main.rs")],
            policy(),
        )
        .unwrap();
        fs::write(workspace.root.join("src/main.rs"), b"later edit").unwrap();
        assert!(bundle.verify().is_ok());
        assert_eq!(
            fs::read(bundle.staging_root().join("src/main.rs")).unwrap(),
            b"synthetic source\n"
        );
    }

    #[test]
    fn should_reject_unselected_or_protected_files_added_to_staging() {
        let workspace = Workspace::new();
        let bundle = SelectedSnapshotBundle::prepare(
            &workspace.metadata,
            &[PathBuf::from("src/main.rs")],
            policy(),
        )
        .unwrap();
        let unexpected = bundle.staging_root().join("unexpected");
        fs::write(&unexpected, b"not selected").unwrap();
        assert_eq!(
            bundle.verify(),
            Err(SnapshotPreparationError::StagingChanged)
        );
        fs::remove_file(unexpected).unwrap();
        fs::write(bundle.staging_root().join(".env"), b"synthetic canary").unwrap();
        assert_eq!(
            bundle.verify(),
            Err(SnapshotPreparationError::StagingChanged)
        );
    }

    #[test]
    fn should_reject_staging_content_changes_and_symlink_replacements() {
        let workspace = Workspace::new();
        let bundle = SelectedSnapshotBundle::prepare(
            &workspace.metadata,
            &[PathBuf::from("src/main.rs")],
            policy(),
        )
        .unwrap();
        fs::write(
            bundle.staging_root().join("src/main.rs"),
            b"modified snapshot",
        )
        .unwrap();
        assert_eq!(
            bundle.verify(),
            Err(SnapshotPreparationError::StagingChanged)
        );
        fs::remove_file(bundle.staging_root().join("src/main.rs")).unwrap();
        symlink(
            workspace.root.join("src/main.rs"),
            bundle.staging_root().join("src/main.rs"),
        )
        .unwrap();
        assert_eq!(
            bundle.verify(),
            Err(SnapshotPreparationError::StagingChanged)
        );
    }

    #[test]
    fn should_refuse_protected_traversal_absolute_and_symlink_source_paths() {
        let workspace = Workspace::new();
        symlink(
            workspace.root.join("src/main.rs"),
            workspace.root.join("linked"),
        )
        .unwrap();
        for path in [".env.example", "../escape", "/outside", "linked"] {
            assert!(
                SelectedSnapshotBundle::prepare(
                    &workspace.metadata,
                    &[PathBuf::from(path)],
                    policy()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn should_reject_duplicate_and_over_budget_selections_without_changing_sources() {
        let workspace = Workspace::new();
        let path = PathBuf::from("src/main.rs");
        assert!(matches!(
            SelectedSnapshotBundle::prepare(
                &workspace.metadata,
                &[path.clone(), path.clone()],
                policy()
            ),
            Err(SnapshotPreparationError::DuplicatePath)
        ));
        assert!(matches!(
            SelectedSnapshotBundle::prepare(
                &workspace.metadata,
                std::slice::from_ref(&path),
                SelectedSnapshotPolicy::new(1, 1).unwrap()
            ),
            Err(SnapshotPreparationError::ByteBudgetExceeded)
        ));
        assert!(matches!(
            SelectedSnapshotBundle::prepare(
                &workspace.metadata,
                &[path, PathBuf::from("not-selected")],
                SelectedSnapshotPolicy::new(1, 1024).unwrap()
            ),
            Err(SnapshotPreparationError::TooManyFiles)
        ));
        assert_eq!(
            fs::read(workspace.root.join("src/main.rs")).unwrap(),
            b"synthetic source\n"
        );
    }

    #[test]
    fn should_keep_the_existing_per_file_read_limit_even_with_a_larger_total_budget() {
        let workspace = Workspace::new();
        fs::write(
            workspace.root.join("large"),
            vec![b'x'; MAX_WORKSPACE_FILE_READ_BYTES + 1],
        )
        .unwrap();
        let selected = [PathBuf::from("large")];
        let policy = SelectedSnapshotPolicy::new(1, MAX_WORKSPACE_FILE_READ_BYTES * 2).unwrap();
        assert!(matches!(
            SelectedSnapshotBundle::prepare(&workspace.metadata, &selected, policy),
            Err(SnapshotPreparationError::SourceRejected)
        ));
        assert_eq!(
            fs::metadata(workspace.root.join("large")).unwrap().len(),
            (MAX_WORKSPACE_FILE_READ_BYTES + 1) as u64
        );
    }

    #[test]
    fn should_keep_selected_snapshots_charged_to_global_quota_until_release() {
        use carapana_storage::{ArtifactKind, ArtifactPolicy, ArtifactStoreError};
        use std::{io::Cursor, time::Duration};
        let workspace = Workspace::new();
        let database = workspace.root.join("data/sessions.sqlite3");
        let quota = ArtifactPolicy::new(
            1024,
            1024,
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .unwrap();
        let mut store = ArtifactStore::open_with_policy(&database, quota).unwrap();
        let bundle = ReservedSelectedSnapshotBundle::prepare(
            &mut store,
            &workspace.metadata,
            &[PathBuf::from("src/main.rs")],
            policy(),
            1_000,
        )
        .unwrap();
        assert_eq!(bundle.file_count(), 1);
        assert_eq!(bundle.total_bytes(), 17);
        assert_eq!(bundle.reserved_bytes(), 1024);
        assert!(bundle.verify().is_ok());
        let root = bundle.staging_root().to_owned();
        let mut other = ArtifactStore::open_with_policy(&database, quota).unwrap();
        assert_eq!(other.usage().unwrap().reserved_bytes, 1024);
        assert!(matches!(
            other.write(ArtifactKind::Durable, 1, Cursor::new(b"x"), 1_000),
            Err(ArtifactStoreError::QuotaExceeded { .. })
        ));
        bundle.release().unwrap();
        assert!(!root.exists());
        assert_eq!(other.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_release_the_global_reservation_when_source_preparation_fails() {
        use carapana_storage::ArtifactPolicy;
        use std::time::Duration;
        let workspace = Workspace::new();
        let database = workspace.root.join("data/sessions.sqlite3");
        let quota = ArtifactPolicy::new(
            1024,
            1024,
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .unwrap();
        let mut store = ArtifactStore::open_with_policy(&database, quota).unwrap();
        assert!(matches!(
            ReservedSelectedSnapshotBundle::prepare(
                &mut store,
                &workspace.metadata,
                &[PathBuf::from("src/main.rs")],
                SelectedSnapshotPolicy::new(1, 1).unwrap(),
                1_000
            ),
            Err(SnapshotPreparationError::ByteBudgetExceeded)
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert_eq!(store.usage().unwrap().artifact_count, 0);
        assert_eq!(
            fs::read(workspace.root.join("src/main.rs")).unwrap(),
            b"synthetic source\n"
        );
    }

    #[test]
    fn should_reject_global_quota_before_attempting_to_read_selected_content() {
        use carapana_storage::{ArtifactKind, ArtifactPolicy};
        use std::{io::Cursor, time::Duration};
        let workspace = Workspace::new();
        let database = workspace.root.join("data/sessions.sqlite3");
        let quota =
            ArtifactPolicy::new(8, 8, Duration::from_millis(100), Duration::from_millis(100))
                .unwrap();
        let mut store = ArtifactStore::open_with_policy(&database, quota).unwrap();
        store
            .write(ArtifactKind::Durable, 8, Cursor::new(b"12345678"), 1_000)
            .unwrap();
        assert!(matches!(
            ReservedSelectedSnapshotBundle::prepare(
                &mut store,
                &workspace.metadata,
                &[PathBuf::from("missing")],
                SelectedSnapshotPolicy::new(1, 4).unwrap(),
                1_000
            ),
            Err(SnapshotPreparationError::ReservationRejected)
        ));
        assert_eq!(store.usage().unwrap().used_bytes, 8);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_require_explicit_valid_limits() {
        for (files, bytes) in [
            (0, 1),
            (1, 0),
            (MAX_SELECTED_FILES + 1, 1),
            (1, MAX_SELECTED_BYTES + 1),
        ] {
            assert_eq!(
                SelectedSnapshotPolicy::new(files, bytes),
                Err(SnapshotPreparationError::InvalidPolicy)
            );
        }
    }
}
