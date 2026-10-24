use super::{
    ArtifactCleanupReport, ArtifactKind, ArtifactStore, ArtifactStoreError, MAX_ARTIFACT_COUNT,
    MAX_ARTIFACT_DIRECTORY_SCAN_ENTRIES, artifact_id_to_i64, blob_name, create_blob_file,
    open_blob_file, try_lock_exclusive, unlink_blob_file, validate_timestamp,
};
use crate::WorkspaceMetadata;
use rusqlite::{OptionalExtension, TransactionBehavior};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// A globally accounted staging lease. Bytes remain reserved until confirmed removal.
///
/// The empty managed blob is a lock/control file; the sibling `.stage` tree contains the staging.
/// Neither is a ready artifact. The full requested budget remains reserved for the lease lifetime.
/// This type contains private paths and deliberately implements neither Debug nor Clone.
/// A future supervisor must retain the lease until every process consuming the staging is reaped;
/// path removal alone cannot prove that an unrelated open descriptor or mount released physical bytes.
pub struct StagingReservation<'store> {
    store: &'store mut ArtifactStore,
    id: u64,
    root: PathBuf,
    identity: WorkspaceMetadata,
    _lock: fs::File,
    reserved_bytes: u64,
    released: bool,
}

impl StagingReservation<'_> {
    pub fn directory(&self) -> &Path {
        &self.root
    }
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    /// Captured identity of this managed tree, not authorization or safe-to-log path metadata.
    pub fn directory_metadata(&self) -> &WorkspaceMetadata {
        &self.identity
    }

    pub fn release(mut self) -> Result<(), ArtifactStoreError> {
        self.release_inner()
    }

    fn release_inner(&mut self) -> Result<(), ArtifactStoreError> {
        if self.released {
            return Ok(());
        }
        match fs::symlink_metadata(&self.root) {
            Ok(_) => self
                .identity
                .verify_current()
                .map_err(|_| ArtifactStoreError::StagingChanged)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ArtifactStoreError::Io(error)),
        }
        self.store.remove_pending(self.id, true)?;
        self.released = true;
        Ok(())
    }
}

impl Drop for StagingReservation<'_> {
    fn drop(&mut self) {
        // A failed cleanup leaves the pending catalog reservation charged for later reconciliation.
        let _ = self.release_inner();
    }
}

impl ArtifactStore {
    pub fn reserve_staging(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
    ) -> Result<StagingReservation<'_>, ArtifactStoreError> {
        validate_timestamp(now_ms)?;
        if max_bytes == 0 {
            return Err(ArtifactStoreError::InvalidByteLimit);
        }
        if max_bytes > self.policy.max_artifact_bytes {
            return Err(ArtifactStoreError::ArtifactTooLarge);
        }
        self.cleanup(now_ms)?;
        let (id, _, lock) = self.reserve(ArtifactKind::Temporary, max_bytes, now_ms)?;
        if let Err(error) = lock.sync_all().and_then(|()| self.directory.sync_all()) {
            self.remove_blob(id)?;
            self.remove_pending(id, false)?;
            return Err(ArtifactStoreError::Io(error));
        }
        if let Err(error) = create_directory(&self.directory, id) {
            // Do not remove a preexisting tree if mkdir failed: this operation never owned it.
            self.remove_blob(id)?;
            self.remove_pending(id, false)?;
            return Err(error);
        }
        let root = self.directory_path.join(directory_name(id));
        let identity = match WorkspaceMetadata::capture(&root) {
            Ok(identity) => identity,
            Err(_) => {
                self.remove_pending(id, true)
                    .map_err(|_| ArtifactStoreError::CleanupFailed(id))?;
                return Err(ArtifactStoreError::StagingChanged);
            }
        };
        if let Err(error) = self.database.connection.execute(
            "INSERT INTO artifact_staging(artifact_id, device, inode) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                artifact_id_to_i64(id)?,
                identity.device().to_be_bytes().as_slice(),
                identity.inode().to_be_bytes().as_slice()
            ],
        ) {
            self.remove_pending(id, true)
                .map_err(|_| ArtifactStoreError::CleanupFailed(id))?;
            return Err(ArtifactStoreError::Sqlite(error));
        }
        if let Err(error) = self.directory.sync_all() {
            self.remove_pending(id, true)
                .map_err(|_| ArtifactStoreError::CleanupFailed(id))?;
            return Err(ArtifactStoreError::Io(error));
        }
        Ok(StagingReservation {
            store: self,
            id,
            root,
            identity,
            _lock: lock,
            reserved_bytes: max_bytes,
            released: false,
        })
    }

    pub(super) fn cleanup_orphan_staging(
        &mut self,
        now_ms: i64,
        report: &mut ArtifactCleanupReport,
    ) -> Result<(), ArtifactStoreError> {
        let known_ids = self.known_artifact_ids()?;
        let ids = find_orphans(
            &self.directory,
            now_ms,
            self.policy.orphan_grace_ms,
            self.policy.cleanup_batch_size,
            &known_ids,
            &mut report.scan_truncated,
        )?;
        for id in ids {
            let file = match open_blob_file(&self.directory, &blob_name(id)) {
                Ok(file) => {
                    if !try_lock_exclusive(&file)? {
                        report.more_work_pending = true;
                        continue;
                    }
                    if !is_old(&file, now_ms, self.policy.orphan_grace_ms)? {
                        continue;
                    }
                    Some(file)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(ArtifactStoreError::Io(error)),
            };
            let directory = &self.directory;
            let directory_path = &self.directory_path;
            let transaction = self
                .database
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifact_records WHERE artifact_id = ?1)",
                [artifact_id_to_i64(id)?],
                |row| row.get(0),
            )?;
            if exists {
                continue;
            }
            let lock = match file {
                Some(file) => file,
                None => {
                    let file = create_blob_file(directory, &blob_name(id))?;
                    if !try_lock_exclusive(&file)? {
                        return Err(ArtifactStoreError::ReconciliationRequired);
                    }
                    file
                }
            };
            if remove_directory(directory, directory_path, id)? {
                report.orphan_staging_directories_removed += 1;
            }
            unlink_blob_file(directory, &blob_name(id))?;
            directory.sync_all()?;
            transaction.commit()?;
            drop(lock);
        }
        Ok(())
    }
}

pub(super) fn directory_name(id: u64) -> String {
    format!("artifact-{id}.stage")
}
pub(super) fn parse_name(name: &str) -> Option<u64> {
    let id = name
        .strip_prefix("artifact-")?
        .strip_suffix(".stage")?
        .parse::<u64>()
        .ok()?;
    if id == 0 || id > i64::MAX as u64 || directory_name(id) != name {
        return None;
    }
    Some(id)
}

fn is_old(file: &fs::File, now_ms: i64, grace_ms: u64) -> Result<bool, ArtifactStoreError> {
    let modified = file
        .metadata()?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
    let modified_ms =
        i64::try_from(modified.as_millis()).map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
    let grace_ms = i64::try_from(grace_ms).map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
    Ok(now_ms.saturating_sub(modified_ms) >= grace_ms)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn entry_exists(parent: &fs::File, id: u64) -> Result<bool, ArtifactStoreError> {
    use rustix::fs::{AtFlags, statat};
    match statat(parent, directory_name(id), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
        Err(error) => Err(ArtifactStoreError::Io(io::Error::from(error))),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_directory(parent: &fs::File, id: u64) -> Result<(), ArtifactStoreError> {
    use rustix::fs::{Mode, mkdirat};
    mkdirat(parent, directory_name(id), Mode::from_bits_truncate(0o700))
        .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn directory_is_old(
    parent: &fs::File,
    id: u64,
    now_ms: i64,
    grace_ms: u64,
) -> Result<bool, ArtifactStoreError> {
    match open_directory(parent, directory_name(id)) {
        Ok(directory) => is_old(&directory, now_ms, grace_ms),
        Err(ArtifactStoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_directory(
    parent: &fs::File,
    name: impl rustix::path::Arg,
) -> Result<fs::File, ArtifactStoreError> {
    use rustix::fs::{Mode, OFlags, openat};
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
    Ok(fs::File::from(fd))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn tree_bytes(parent: &fs::File, id: u64) -> Result<u64, ArtifactStoreError> {
    use rustix::fs::{AtFlags, Dir, FileType, statat};
    let root = match open_directory(parent, directory_name(id)) {
        Ok(root) => root,
        Err(ArtifactStoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let mut pending = vec![root];
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut stream = Dir::new(open_directory(&directory, ".")?)
            .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        while let Some(entry) = stream.read() {
            let entry = entry.map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
            if matches!(entry.file_name().to_bytes(), b"." | b"..") {
                continue;
            }
            entries += 1;
            if entries > MAX_ARTIFACT_COUNT as usize {
                return Err(ArtifactStoreError::ReconciliationRequired);
            }
            let stat = statat(&directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => pending.push(open_directory(&directory, entry.file_name())?),
                FileType::RegularFile | FileType::Symlink => {
                    let size = u64::try_from(stat.st_size)
                        .map_err(|_| ArtifactStoreError::InvalidDatabaseState)?;
                    bytes = bytes
                        .checked_add(size)
                        .ok_or(ArtifactStoreError::InvalidDatabaseState)?;
                }
                _ => return Err(ArtifactStoreError::ReconciliationRequired),
            }
        }
    }
    Ok(bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn remove_directory(
    parent: &fs::File,
    parent_path: &Path,
    id: u64,
) -> Result<bool, ArtifactStoreError> {
    use std::os::unix::fs::MetadataExt;
    let opened = parent.metadata()?;
    let path_metadata = fs::symlink_metadata(parent_path)?;
    if !path_metadata.is_dir()
        || opened.dev() != path_metadata.dev()
        || opened.ino() != path_metadata.ino()
    {
        return Err(ArtifactStoreError::StagingChanged);
    }
    let root = parent_path.join(directory_name(id));
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ArtifactStoreError::Io(error)),
    };
    if !metadata.is_dir() {
        return Err(ArtifactStoreError::StagingChanged);
    }
    // Bound traversal before recursive removal; never follow a symlink to determine usage.
    tree_bytes(parent, id)?;
    fs::remove_dir_all(root)?;
    parent.sync_all()?;
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn remove_registered_directory(
    connection: &rusqlite::Connection,
    parent: &fs::File,
    parent_path: &Path,
    id: u64,
) -> Result<bool, ArtifactStoreError> {
    use rustix::fs::Dir;
    use std::os::unix::fs::MetadataExt;
    let directory = match open_directory(parent, directory_name(id)) {
        Ok(directory) => directory,
        Err(ArtifactStoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let expected: Option<(Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT device, inode FROM artifact_staging WHERE artifact_id = ?1",
            [artifact_id_to_i64(id)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((device, inode)) = expected {
        let device: [u8; 8] = device
            .try_into()
            .map_err(|_| ArtifactStoreError::InvalidDatabaseState)?;
        let inode: [u8; 8] = inode
            .try_into()
            .map_err(|_| ArtifactStoreError::InvalidDatabaseState)?;
        let metadata = directory.metadata()?;
        if metadata.dev() != u64::from_be_bytes(device)
            || metadata.ino() != u64::from_be_bytes(inode)
        {
            return Err(ArtifactStoreError::StagingChanged);
        }
    } else {
        // A crash before identity publication can leave only an empty tree. Never adopt nonempty
        // unregistered content as a known reservation's tree merely because its name matches.
        let mut stream = Dir::new(open_directory(&directory, ".")?)
            .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        while let Some(entry) = stream.read() {
            let entry = entry.map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
            if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                return Err(ArtifactStoreError::StagingChanged);
            }
        }
    }
    remove_directory(parent, parent_path, id)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn find_orphans(
    parent: &fs::File,
    now_ms: i64,
    grace_ms: u64,
    batch_size: usize,
    known_ids: &std::collections::HashSet<u64>,
    truncated: &mut bool,
) -> Result<Vec<u64>, ArtifactStoreError> {
    use rustix::fs::Dir;
    let mut stream = Dir::new(open_directory(parent, ".")?)
        .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
    let mut entries = 0_usize;
    let mut ids = Vec::new();
    while let Some(entry) = stream.read() {
        let entry = entry.map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        if matches!(entry.file_name().to_bytes(), b"." | b"..") {
            continue;
        }
        entries += 1;
        if entries > MAX_ARTIFACT_DIRECTORY_SCAN_ENTRIES {
            *truncated = true;
            break;
        }
        let Ok(name) = std::str::from_utf8(entry.file_name().to_bytes()) else {
            continue;
        };
        let Some(id) = parse_name(name) else {
            continue;
        };
        if known_ids.contains(&id) {
            continue;
        }
        let directory = open_directory(parent, entry.file_name())?;
        if is_old(&directory, now_ms, grace_ms)? {
            ids.push(id);
            if ids.len() >= batch_size {
                *truncated = true;
                break;
            }
        }
    }
    Ok(ids)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn entry_exists(_parent: &fs::File, _id: u64) -> Result<bool, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{ArtifactPolicy, SCHEMA_VERSION};
    use std::{
        io::Cursor,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "carapana-staging-{}-{nonce}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn database(&self) -> PathBuf {
            self.0.join("sessions.sqlite3")
        }
        fn store(&self, bytes: u64) -> ArtifactStore {
            ArtifactStore::open_with_policy(
                self.database(),
                ArtifactPolicy::new(
                    bytes,
                    bytes,
                    Duration::from_millis(100),
                    Duration::from_millis(100),
                )
                .unwrap(),
            )
            .unwrap()
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn write_private(path: &Path, bytes: &[u8]) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn should_share_quota_between_artifacts_and_live_staging_across_connections() {
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let durable = store
            .write(ArtifactKind::Durable, 4, Cursor::new(b"keep"), 1_000)
            .unwrap();
        let reservation = store.reserve_staging(4, 1_000).unwrap();
        let path = reservation.directory().to_owned();
        write_private(&path.join("selected"), b"data");
        let mut second = root.store(8);
        assert_eq!(second.usage().unwrap().reserved_bytes, 4);
        assert!(matches!(
            second.write(ArtifactKind::Durable, 1, Cursor::new(b"x"), 1_000),
            Err(ArtifactStoreError::QuotaExceeded { .. })
        ));
        assert_eq!(second.list(None, 10, 1_000).unwrap(), vec![durable]);
        reservation.release().unwrap();
        assert!(!path.exists());
        assert_eq!(second.usage().unwrap().reserved_bytes, 0);
        assert!(
            second
                .write(ArtifactKind::Durable, 1, Cursor::new(b"x"), 1_000)
                .is_ok()
        );
    }

    #[test]
    fn should_not_reclaim_live_staging_even_after_the_grace_period() {
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let reservation = store.reserve_staging(8, 1_000).unwrap();
        write_private(&reservation.directory().join("selected"), b"data");
        let mut second = root.store(8);
        let report = second.cleanup(1_200).unwrap();
        assert_eq!(report.pending_writes_removed, 0);
        assert_eq!(second.usage().unwrap().reserved_bytes, 8);
        assert!(reservation.directory().join("selected").exists());
        drop(reservation);
        assert_eq!(second.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_recover_an_abandoned_staging_lease_after_grace_without_evicting_durable_data() {
        let root = TestDirectory::new();
        let mut store = root.store(16);
        let durable = store
            .write(ArtifactKind::Durable, 4, Cursor::new(b"keep"), 1_000)
            .unwrap();
        let mut reservation = store.reserve_staging(8, 1_000).unwrap();
        let path = reservation.directory().to_owned();
        write_private(&path.join("selected"), b"data");
        // Simulate process loss: release the OS lock without running the normal cleanup protocol.
        reservation.released = true;
        drop(reservation);
        assert_eq!(store.cleanup(1_050).unwrap().pending_writes_removed, 0);
        assert_eq!(store.usage().unwrap().reserved_bytes, 8);
        assert_eq!(store.cleanup(1_200).unwrap().pending_writes_removed, 1);
        assert!(!path.exists());
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert_eq!(store.list(None, 10, 1_200).unwrap(), vec![durable]);
    }

    #[test]
    fn should_account_and_clean_an_orphan_staging_tree_without_a_control_blob() {
        use std::fs::FileTimes;
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let path = store.directory_path.join(directory_name(900));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        write_private(&path.join("selected"), b"data");
        fs::File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        assert_eq!(store.usage().unwrap().orphan_bytes, 4);
        assert!(matches!(
            store.write(ArtifactKind::Durable, 8, Cursor::new(b"new"), 10),
            Err(ArtifactStoreError::QuotaExceeded { .. })
        ));
        assert_eq!(
            store
                .cleanup(1_000)
                .unwrap()
                .orphan_staging_directories_removed,
            1
        );
        assert!(!path.exists());
        assert_eq!(store.usage().unwrap().orphan_bytes, 0);
    }

    #[test]
    fn should_not_expire_a_recent_orphan_tree_just_because_its_control_blob_is_old() {
        use std::fs::FileTimes;
        let root = TestDirectory::new();
        let mut store = root.store(64);
        let id = 902;
        let blob = store.directory_path.join(blob_name(id));
        write_private(&blob, b"");
        fs::File::open(&blob)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        let path = store.directory_path.join(directory_name(id));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        write_private(&path.join("selected"), b"data");
        fs::File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(950)))
            .unwrap();
        store.cleanup(1_000).unwrap();
        assert!(
            path.exists(),
            "recent staging was removed through an older orphan blob"
        );
        assert_eq!(store.usage().unwrap().orphan_bytes, 4);
        store.cleanup(1_200).unwrap();
        assert!(!path.exists());
        assert!(!blob.exists());
    }

    #[test]
    fn should_preserve_replaced_staging_identity_and_keep_its_quota_charged() {
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let reservation = store.reserve_staging(8, 1_000).unwrap();
        let path = reservation.directory().to_owned();
        let moved = path.with_extension("moved");
        fs::rename(&path, &moved).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        write_private(&path.join("preserve"), b"work");
        assert!(matches!(
            reservation.release(),
            Err(ArtifactStoreError::StagingChanged)
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 8);
        assert!(matches!(
            store.cleanup(1_200),
            Err(ArtifactStoreError::StagingChanged)
        ));
        assert_eq!(fs::read(path.join("preserve")).unwrap(), b"work");
        assert!(moved.exists());
    }

    #[test]
    fn should_not_follow_a_symlink_when_removing_orphan_staging_contents() {
        use std::{fs::FileTimes, os::unix::fs::symlink};
        let root = TestDirectory::new();
        let mut store = root.store(1024);
        let outside = root.0.join("outside");
        write_private(&outside, b"preserve outside");
        let path = store.directory_path.join(directory_name(901));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        symlink(&outside, path.join("linked")).unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        store.cleanup(1_000).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read(outside).unwrap(), b"preserve outside");
    }

    #[test]
    fn should_recover_an_empty_tree_after_a_crash_before_identity_publication() {
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let mut reservation = store.reserve_staging(8, 1_000).unwrap();
        let id = reservation.id;
        let path = reservation.directory().to_owned();
        reservation.released = true;
        drop(reservation);
        store
            .database
            .connection
            .execute(
                "DELETE FROM artifact_staging WHERE artifact_id = ?1",
                [id as i64],
            )
            .unwrap();
        assert_eq!(store.cleanup(1_200).unwrap().pending_writes_removed, 1);
        assert!(!path.exists());
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_not_adopt_nonempty_unregistered_content_as_a_known_staging_reservation() {
        let root = TestDirectory::new();
        let mut store = root.store(8);
        let mut reservation = store.reserve_staging(8, 1_000).unwrap();
        let id = reservation.id;
        let path = reservation.directory().to_owned();
        write_private(&path.join("preserve"), b"work");
        reservation.released = true;
        drop(reservation);
        store
            .database
            .connection
            .execute(
                "DELETE FROM artifact_staging WHERE artifact_id = ?1",
                [id as i64],
            )
            .unwrap();
        assert!(matches!(
            store.cleanup(1_200),
            Err(ArtifactStoreError::StagingChanged)
        ));
        assert_eq!(fs::read(path.join("preserve")).unwrap(), b"work");
        assert_eq!(store.usage().unwrap().reserved_bytes, 8);
    }

    #[test]
    fn should_migrate_version_six_before_writing_staging_identity_metadata() {
        let root = TestDirectory::new();
        let store = root.store(8);
        store
            .database
            .connection
            .execute_batch("DROP TABLE artifact_staging; PRAGMA user_version = 6;")
            .unwrap();
        drop(store);
        let mut store = root.store(8);
        assert_eq!(store.database.schema_version().unwrap(), SCHEMA_VERSION);
        let reservation = store.reserve_staging(8, 1_000).unwrap();
        reservation.release().unwrap();
        let rows: i64 = store
            .database
            .connection
            .query_row("SELECT COUNT(*) FROM artifact_staging", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_directory(_parent: &fs::File, _id: u64) -> Result<(), ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn directory_is_old(
    _parent: &fs::File,
    _id: u64,
    _now_ms: i64,
    _grace_ms: u64,
) -> Result<bool, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn tree_bytes(_parent: &fs::File, _id: u64) -> Result<u64, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn remove_directory(
    _parent: &fs::File,
    _path: &Path,
    _id: u64,
) -> Result<bool, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn remove_registered_directory(
    _connection: &rusqlite::Connection,
    _parent: &fs::File,
    _path: &Path,
    _id: u64,
) -> Result<bool, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn find_orphans(
    _parent: &fs::File,
    _now_ms: i64,
    _grace_ms: u64,
    _batch: usize,
    _known: &std::collections::HashSet<u64>,
    _truncated: &mut bool,
) -> Result<Vec<u64>, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}
