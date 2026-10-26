use crate::{
    Database, MigrationError, UserDatabaseError,
    user_database::{
        DATABASE_FILE, ensure_private_database_file, ensure_private_directory, user_data_directory,
        user_database_path,
    },
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt, fs, io,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

mod staging;
pub use staging::StagingReservation;

const ARTIFACT_DIRECTORY_NAME: &str = "artifacts";
const MAX_ARTIFACT_COUNT: i64 = 16_384;
const MAX_CLEANUP_BATCH_SIZE: usize = 128;
const MAX_ARTIFACT_ID_ALLOCATION_ATTEMPTS: usize = 128;
const MAX_ARTIFACT_DIRECTORY_SCAN_ENTRIES: usize =
    2 * MAX_ARTIFACT_COUNT as usize + MAX_CLEANUP_BATCH_SIZE;
const MAX_LIST_PAGE_SIZE: u16 = 256;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

pub const DEFAULT_ARTIFACT_STORE_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_ARTIFACT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const DEFAULT_ORPHAN_GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Configurable admission limits for the isolated per-user artifact store.
///
/// The accepted defaults are ceilings, not hardware-calibrated recommendations. A policy may lower
/// them; increasing either ceiling requires revisiting PD-32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactPolicy {
    max_total_bytes: u64,
    max_artifact_bytes: u64,
    temporary_retention_ms: u64,
    orphan_grace_ms: u64,
    cleanup_batch_size: usize,
}

impl ArtifactPolicy {
    pub fn new(
        max_total_bytes: u64,
        max_artifact_bytes: u64,
        temporary_retention: Duration,
        orphan_grace: Duration,
    ) -> Result<Self, ArtifactStoreError> {
        let temporary_retention_ms = duration_millis(temporary_retention)?;
        let orphan_grace_ms = duration_millis(orphan_grace)?;
        if max_total_bytes == 0
            || max_total_bytes > DEFAULT_ARTIFACT_STORE_QUOTA_BYTES
            || max_artifact_bytes == 0
            || max_artifact_bytes > DEFAULT_MAX_ARTIFACT_BYTES
            || max_artifact_bytes > max_total_bytes
            || temporary_retention_ms == 0
            || orphan_grace_ms == 0
        {
            return Err(ArtifactStoreError::InvalidPolicy);
        }

        Ok(Self {
            max_total_bytes,
            max_artifact_bytes,
            temporary_retention_ms,
            orphan_grace_ms,
            cleanup_batch_size: MAX_CLEANUP_BATCH_SIZE,
        })
    }

    pub fn max_total_bytes(self) -> u64 {
        self.max_total_bytes
    }

    pub fn max_artifact_bytes(self) -> u64 {
        self.max_artifact_bytes
    }

    pub fn temporary_retention(self) -> Duration {
        Duration::from_millis(self.temporary_retention_ms)
    }

    pub fn orphan_grace(self) -> Duration {
        Duration::from_millis(self.orphan_grace_ms)
    }

    pub fn with_cleanup_batch_size(
        mut self,
        cleanup_batch_size: usize,
    ) -> Result<Self, ArtifactStoreError> {
        if cleanup_batch_size == 0 || cleanup_batch_size > MAX_CLEANUP_BATCH_SIZE {
            return Err(ArtifactStoreError::InvalidPolicy);
        }
        self.cleanup_batch_size = cleanup_batch_size;
        Ok(self)
    }
}

impl Default for ArtifactPolicy {
    fn default() -> Self {
        Self {
            max_total_bytes: DEFAULT_ARTIFACT_STORE_QUOTA_BYTES,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            temporary_retention_ms: DEFAULT_ARTIFACT_RETENTION.as_millis() as u64,
            orphan_grace_ms: DEFAULT_ORPHAN_GRACE.as_millis() as u64,
            cleanup_batch_size: MAX_CLEANUP_BATCH_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    Durable,
    Temporary,
}

impl ArtifactKind {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Temporary => "temporary",
        }
    }

    fn from_db_value(value: &str) -> Result<Self, ArtifactStoreError> {
        match value {
            "durable" => Ok(Self::Durable),
            "temporary" => Ok(Self::Temporary),
            _ => Err(ArtifactStoreError::InvalidDatabaseState),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactMetadata {
    pub id: u64,
    pub kind: ArtifactKind,
    pub size_bytes: u64,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactUsage {
    pub used_bytes: u64,
    /// Includes live and abandoned staging reservations until confirmed cleanup.
    pub reserved_bytes: u64,
    /// Managed bytes not covered by a catalog entry or staging reservation, retained until cleanup.
    pub orphan_bytes: u64,
    /// Catalog records, including pending writes and staging leases, not only ready artifacts.
    pub artifact_count: u64,
    pub quota_bytes: u64,
    pub max_artifact_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArtifactCleanupReport {
    pub temporary_artifacts_removed: u64,
    pub pending_writes_removed: u64,
    pub deleting_records_reconciled: u64,
    pub orphan_files_removed: u64,
    pub orphan_staging_directories_removed: u64,
    /// True when the bounded directory scan stopped before reaching the end.
    pub scan_truncated: bool,
    /// True when cleanup records or eligible files remain for a later batch/retry.
    pub more_work_pending: bool,
}

#[derive(Debug)]
pub enum ArtifactStoreError {
    UserDatabase(UserDatabaseError),
    Migration(MigrationError),
    Sqlite(rusqlite::Error),
    Io(io::Error),
    InvalidPolicy,
    InvalidTimestamp,
    InvalidByteLimit,
    ArtifactTooLarge,
    ArtifactCountExceeded,
    ArtifactIdAllocationExhausted,
    ReconciliationRequired,
    StagingChanged,
    QuotaExceeded {
        quota_bytes: u64,
        used_bytes: u64,
        requested_bytes: u64,
    },
    ArtifactNotFound(u64),
    ArtifactNotReady(u64),
    ArtifactExpired,
    ArtifactContentMissing,
    ArtifactIntegrityMismatch,
    InvalidDatabaseState,
    CommitOutcomeUncertain(u64),
    CleanupFailed(u64),
    UnsupportedPlatform,
}

impl fmt::Display for ArtifactStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserDatabase(error) => {
                write!(formatter, "artifact store path is unsafe: {error}")
            }
            Self::Migration(error) => write!(
                formatter,
                "artifact store database initialization failed: {error}"
            ),
            Self::Sqlite(error) => write!(
                formatter,
                "artifact store metadata operation failed: {error}"
            ),
            Self::Io(error) => write!(
                formatter,
                "artifact store filesystem operation failed: {error}"
            ),
            Self::InvalidPolicy => formatter.write_str("artifact store policy is invalid"),
            Self::InvalidTimestamp => formatter.write_str("artifact store timestamp is invalid"),
            Self::InvalidByteLimit => formatter.write_str("artifact write limit must be positive"),
            Self::ArtifactTooLarge => {
                formatter.write_str("artifact exceeds the configured size limit")
            }
            Self::ArtifactCountExceeded => {
                formatter.write_str("artifact store has reached its item limit")
            }
            Self::ArtifactIdAllocationExhausted => {
                formatter.write_str("artifact ID allocation encountered too many existing files; reconcile the store before retrying")
            }
            Self::ReconciliationRequired => {
                formatter.write_str("artifact directory exceeds the bounded accounting scan; reconcile the store before writing")
            }
            Self::StagingChanged => formatter.write_str("staging identity changed; quota remains reserved until safe reconciliation"),
            Self::QuotaExceeded {
                quota_bytes,
                used_bytes,
                requested_bytes,
            } => write!(
                formatter,
                "artifact quota exceeded: {used_bytes} bytes used, {requested_bytes} requested, {quota_bytes} byte limit"
            ),
            Self::ArtifactNotFound(id) => write!(formatter, "artifact {id} was not found"),
            Self::ArtifactNotReady(id) => write!(formatter, "artifact {id} is not ready"),
            Self::ArtifactExpired => formatter.write_str("temporary artifact has expired"),
            Self::ArtifactContentMissing => formatter.write_str("artifact content is missing"),
            Self::ArtifactIntegrityMismatch => {
                formatter.write_str("artifact content failed integrity verification")
            }
            Self::InvalidDatabaseState => formatter.write_str("artifact store metadata is invalid"),
            Self::CommitOutcomeUncertain(id) => write!(
                formatter,
                "artifact {id} write outcome is uncertain; inspect the artifact catalog before retrying"
            ),
            Self::CleanupFailed(id) => write!(
                formatter,
                "artifact {id} write failed and pending cleanup could not be completed"
            ),
            Self::UnsupportedPlatform => {
                formatter.write_str("artifact store is supported only on Linux and macOS")
            }
        }
    }
}

impl Error for ArtifactStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UserDatabase(error) => Some(error),
            Self::Migration(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for ArtifactStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<io::Error> for ArtifactStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<UserDatabaseError> for ArtifactStoreError {
    fn from(error: UserDatabaseError) -> Self {
        Self::UserDatabase(error)
    }
}

impl From<MigrationError> for ArtifactStoreError {
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}

/// A bounded, local artifact catalog backed by SQLite and a private file directory.
///
/// This API is intentionally isolated from the daemon, tools, provider, and session queue. It does
/// not classify content; callers must enforce their own secret and authorization gates.
pub struct ArtifactStore {
    database: Database,
    directory: fs::File,
    directory_path: PathBuf,
    policy: ArtifactPolicy,
}

impl ArtifactStore {
    pub fn open_user() -> Result<Self, ArtifactStoreError> {
        Self::open_user_with_policy(ArtifactPolicy::default())
    }

    pub fn open_user_with_policy(policy: ArtifactPolicy) -> Result<Self, ArtifactStoreError> {
        user_data_directory()?;
        Self::open_with_policy(user_database_path()?, policy)
    }

    /// Open the single `sessions.sqlite3` catalog for a private user-data directory.
    /// Other filenames are rejected so independent catalogs cannot share and clean each other's blobs.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArtifactStoreError> {
        Self::open_with_policy(path, ArtifactPolicy::default())
    }

    pub fn open_with_policy(
        path: impl AsRef<Path>,
        policy: ArtifactPolicy,
    ) -> Result<Self, ArtifactStoreError> {
        let database_path = path.as_ref();
        if database_path.file_name() != Some(std::ffi::OsStr::new(DATABASE_FILE)) {
            return Err(ArtifactStoreError::UserDatabase(
                UserDatabaseError::UnsafePath(database_path.to_owned()),
            ));
        }
        let parent = database_path.parent().ok_or_else(|| {
            ArtifactStoreError::UserDatabase(UserDatabaseError::UnsafePath(
                database_path.to_owned(),
            ))
        })?;
        ensure_private_directory(parent)?;
        ensure_private_database_file(database_path)?;

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let database = Database::open(database_path)?;
            let artifact_path = parent.join(ARTIFACT_DIRECTORY_NAME);
            ensure_private_directory(&artifact_path)?;
            let directory = open_private_directory(&artifact_path)?;
            Ok(Self {
                database,
                directory,
                directory_path: artifact_path,
                policy,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (database_path, policy);
            Err(ArtifactStoreError::UnsupportedPlatform)
        }
    }

    /// Stream an artifact to private storage after reserving its maximum byte budget in SQLite.
    pub fn write<R: Read>(
        &mut self,
        kind: ArtifactKind,
        max_bytes: u64,
        mut source: R,
        now_ms: i64,
    ) -> Result<ArtifactMetadata, ArtifactStoreError> {
        validate_timestamp(now_ms)?;
        if max_bytes == 0 {
            return Err(ArtifactStoreError::InvalidByteLimit);
        }
        if max_bytes > self.policy.max_artifact_bytes {
            return Err(ArtifactStoreError::ArtifactTooLarge);
        }

        self.cleanup(now_ms)?;
        let (artifact_id, expires_at_ms, mut file) = self.reserve(kind, max_bytes, now_ms)?;

        let (size_bytes, digest) = match copy_bounded(&mut source, &mut file, max_bytes) {
            Ok(result) => result,
            Err(error) => {
                self.remove_pending(artifact_id, true)
                    .map_err(|_| ArtifactStoreError::CleanupFailed(artifact_id))?;
                drop(file);
                return Err(error);
            }
        };
        if let Err(error) = file.sync_all() {
            self.remove_pending(artifact_id, true)
                .map_err(|_| ArtifactStoreError::CleanupFailed(artifact_id))?;
            drop(file);
            return Err(ArtifactStoreError::Io(error));
        }
        if let Err(error) = self.directory.sync_all() {
            self.remove_pending(artifact_id, true)
                .map_err(|_| ArtifactStoreError::CleanupFailed(artifact_id))?;
            drop(file);
            return Err(ArtifactStoreError::Io(error));
        }

        self.finalize(artifact_id, size_bytes, &digest)?;
        drop(file);
        Ok(ArtifactMetadata {
            id: artifact_id,
            kind,
            size_bytes,
            created_at_ms: now_ms,
            expires_at_ms,
        })
    }

    /// Verify stored bytes before copying them to a caller-owned sink.
    ///
    /// A sink may receive partial bytes if its own write fails; callers must not expose those bytes
    /// unless this method returns successfully.
    pub fn copy_to<W: Write>(
        &self,
        artifact_id: u64,
        now_ms: i64,
        destination: &mut W,
    ) -> Result<ArtifactMetadata, ArtifactStoreError> {
        validate_timestamp(now_ms)?;
        let record = self.load_record(artifact_id)?;
        if record.state != "ready" {
            return Err(ArtifactStoreError::ArtifactNotReady(artifact_id));
        }
        let metadata = record.metadata()?;
        if metadata
            .expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
        {
            return Err(ArtifactStoreError::ArtifactExpired);
        }
        let expected_digest = record
            .sha256
            .ok_or(ArtifactStoreError::InvalidDatabaseState)?;
        let file_name = blob_name(artifact_id);
        let mut file =
            open_blob_file(&self.directory, &file_name).map_err(map_content_open_error)?;
        lock_shared(&file)?;
        let current_record = self.load_record(artifact_id)?;
        if current_record.state != "ready"
            || current_record.sha256.as_deref() != Some(expected_digest.as_slice())
            || current_record.size_bytes != record.size_bytes
        {
            return Err(ArtifactStoreError::ArtifactNotReady(artifact_id));
        }
        validate_private_artifact_file(&file, metadata.size_bytes)?;

        let mut hasher = Sha256::new();
        let mut observed_size = 0_u64;
        let mut buffer = [0_u8; COPY_BUFFER_BYTES];
        loop {
            let count = match file.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ArtifactStoreError::Io(error)),
            };
            if count == 0 {
                break;
            }
            observed_size = observed_size
                .checked_add(count as u64)
                .ok_or(ArtifactStoreError::ArtifactIntegrityMismatch)?;
            if observed_size > metadata.size_bytes {
                return Err(ArtifactStoreError::ArtifactIntegrityMismatch);
            }
            hasher.update(&buffer[..count]);
        }
        let actual_digest = hasher.finalize();
        if observed_size != metadata.size_bytes
            || actual_digest.as_slice() != expected_digest.as_slice()
        {
            return Err(ArtifactStoreError::ArtifactIntegrityMismatch);
        }

        file.seek(SeekFrom::Start(0))?;
        let mut copied_size = 0_u64;
        let mut copied_hasher = Sha256::new();
        loop {
            let count = match file.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ArtifactStoreError::Io(error)),
            };
            if count == 0 {
                break;
            }
            copied_size = copied_size
                .checked_add(count as u64)
                .ok_or(ArtifactStoreError::ArtifactIntegrityMismatch)?;
            if copied_size > metadata.size_bytes {
                return Err(ArtifactStoreError::ArtifactIntegrityMismatch);
            }
            copied_hasher.update(&buffer[..count]);
            destination.write_all(&buffer[..count])?;
        }
        if copied_size != metadata.size_bytes
            || copied_hasher.finalize().as_slice() != expected_digest.as_slice()
        {
            return Err(ArtifactStoreError::ArtifactIntegrityMismatch);
        }
        Ok(metadata)
    }

    pub fn list(
        &self,
        after_id: Option<u64>,
        limit: u16,
        now_ms: i64,
    ) -> Result<Vec<ArtifactMetadata>, ArtifactStoreError> {
        validate_timestamp(now_ms)?;
        if limit == 0 || limit > MAX_LIST_PAGE_SIZE {
            return Err(ArtifactStoreError::InvalidPolicy);
        }
        let after_id = after_id.map(artifact_id_to_i64).transpose()?.unwrap_or(0);
        let mut statement = self.database.connection.prepare(
            "SELECT artifact_id, kind, state, size_bytes, reserved_bytes, created_at_ms, expires_at_ms, sha256 \
             FROM artifact_records WHERE state = 'ready' AND artifact_id > ?1 \
             AND (kind = 'durable' OR expires_at_ms > ?2) ORDER BY artifact_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            rusqlite::params![after_id, now_ms, i64::from(limit)],
            ArtifactRecord::from_row,
        )?;
        let mut artifacts = Vec::new();
        for row in rows {
            let record = row?;
            artifacts.push(record.metadata()?);
        }
        Ok(artifacts)
    }

    pub fn usage(&self) -> Result<ArtifactUsage, ArtifactStoreError> {
        let (used_bytes, reserved_bytes, artifact_count): (i64, i64, i64) =
            self.database.connection.query_row(
                "SELECT \
                    COALESCE(SUM(CASE WHEN state IN ('ready', 'deleting') THEN size_bytes ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN state = 'pending' THEN reserved_bytes ELSE 0 END), 0), \
                    COUNT(*) \
                 FROM artifact_records",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let orphan_bytes = managed_orphan_bytes(
            &self.directory,
            &self.known_artifact_ids()?,
            &catalog_pending_ids(&self.database.connection)?,
        )?;
        Ok(ArtifactUsage {
            used_bytes: nonnegative_u64(used_bytes)?,
            reserved_bytes: nonnegative_u64(reserved_bytes)?,
            orphan_bytes,
            artifact_count: nonnegative_u64(artifact_count)?,
            quota_bytes: self.policy.max_total_bytes,
            max_artifact_bytes: self.policy.max_artifact_bytes,
        })
    }

    pub fn delete(&mut self, artifact_id: u64) -> Result<(), ArtifactStoreError> {
        let artifact_id = artifact_id_to_i64(artifact_id)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE artifact_records SET state = 'deleting' WHERE artifact_id = ?1 AND state = 'ready'",
            [artifact_id],
        )?;
        if changed == 0 {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifact_records WHERE artifact_id = ?1)",
                [artifact_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(ArtifactStoreError::ArtifactNotFound(artifact_id as u64));
            }
            return Err(ArtifactStoreError::ArtifactNotReady(artifact_id as u64));
        }
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id as u64))?;

        self.finish_deleting(artifact_id as u64)?;
        Ok(())
    }

    pub fn cleanup(&mut self, now_ms: i64) -> Result<ArtifactCleanupReport, ArtifactStoreError> {
        validate_timestamp(now_ms)?;
        let mut report = ArtifactCleanupReport::default();
        self.cleanup_deleting(&mut report)?;
        self.cleanup_expired_temporary(now_ms, &mut report)?;
        self.cleanup_stale_pending(now_ms, &mut report)?;
        self.cleanup_orphan_files(now_ms, &mut report)?;
        self.cleanup_orphan_staging(now_ms, &mut report)?;
        report.more_work_pending |=
            report.scan_truncated || self.has_cleanup_work_remaining(now_ms)?;
        Ok(report)
    }

    fn has_cleanup_work_remaining(&self, now_ms: i64) -> Result<bool, ArtifactStoreError> {
        let deleting_or_expired: bool = self.database.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM artifact_records WHERE state = 'deleting') \
             OR EXISTS(SELECT 1 FROM artifact_records WHERE state = 'ready' AND kind = 'temporary' AND expires_at_ms <= ?1)",
            [now_ms],
            |row| row.get(0),
        )?;
        if deleting_or_expired {
            return Ok(true);
        }
        let grace_ms = i64::try_from(self.policy.orphan_grace_ms)
            .map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
        let Some(cutoff_ms) = now_ms.checked_sub(grace_ms) else {
            return Ok(false);
        };
        let stale_pending: bool = self.database.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM artifact_records WHERE state = 'pending' AND created_at_ms <= ?1)",
            [cutoff_ms],
            |row| row.get(0),
        )?;
        Ok(stale_pending)
    }

    fn reserve(
        &mut self,
        kind: ArtifactKind,
        max_bytes: u64,
        now_ms: i64,
    ) -> Result<(u64, Option<i64>, fs::File), ArtifactStoreError> {
        let expires_at_ms = match kind {
            ArtifactKind::Durable => None,
            ArtifactKind::Temporary => Some(
                now_ms
                    .checked_add(
                        i64::try_from(self.policy.temporary_retention_ms)
                            .map_err(|_| ArtifactStoreError::InvalidTimestamp)?,
                    )
                    .ok_or(ArtifactStoreError::InvalidTimestamp)?,
            ),
        };
        let max_bytes_i64 =
            i64::try_from(max_bytes).map_err(|_| ArtifactStoreError::ArtifactTooLarge)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (used_bytes, reserved_bytes, artifact_count): (i64, i64, i64) = transaction.query_row(
            "SELECT \
                COALESCE(SUM(CASE WHEN state IN ('ready', 'deleting') THEN size_bytes ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN state = 'pending' THEN reserved_bytes ELSE 0 END), 0), \
                COUNT(*) \
             FROM artifact_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if artifact_count >= MAX_ARTIFACT_COUNT {
            return Err(ArtifactStoreError::ArtifactCountExceeded);
        }
        // Hold the catalog write transaction while accounting unregistered blobs: another store
        // cannot create a new pending blob between this scan and reservation publication.
        let known_ids = catalog_artifact_ids(&transaction)?;
        let pending_ids = catalog_pending_ids(&transaction)?;
        let orphan_bytes = managed_orphan_bytes(&self.directory, &known_ids, &pending_ids)?;
        let orphan_bytes_i64 =
            i64::try_from(orphan_bytes).map_err(|_| ArtifactStoreError::InvalidDatabaseState)?;
        let used_total = used_bytes
            .checked_add(reserved_bytes)
            .and_then(|value| value.checked_add(orphan_bytes_i64))
            .and_then(|value| value.checked_add(max_bytes_i64))
            .ok_or(ArtifactStoreError::QuotaExceeded {
                quota_bytes: self.policy.max_total_bytes,
                used_bytes: nonnegative_u64(
                    used_bytes
                        .saturating_add(reserved_bytes)
                        .saturating_add(orphan_bytes_i64),
                )?,
                requested_bytes: max_bytes,
            })?;
        let used_total =
            u64::try_from(used_total).map_err(|_| ArtifactStoreError::InvalidDatabaseState)?;
        if used_total > self.policy.max_total_bytes {
            return Err(ArtifactStoreError::QuotaExceeded {
                quota_bytes: self.policy.max_total_bytes,
                used_bytes: nonnegative_u64(
                    used_bytes
                        .saturating_add(reserved_bytes)
                        .saturating_add(orphan_bytes_i64),
                )?,
                requested_bytes: max_bytes,
            });
        }

        for _ in 0..MAX_ARTIFACT_ID_ALLOCATION_ATTEMPTS {
            transaction.execute(
                "INSERT INTO artifact_records (kind, state, size_bytes, reserved_bytes, created_at_ms, expires_at_ms, sha256) \
                 VALUES (?1, 'pending', 0, ?2, ?3, ?4, NULL)",
                rusqlite::params![kind.as_db_value(), max_bytes_i64, now_ms, expires_at_ms],
            )?;
            let artifact_id = nonnegative_u64(transaction.last_insert_rowid())?;
            if artifact_id == 0 {
                return Err(ArtifactStoreError::InvalidDatabaseState);
            }
            if staging::entry_exists(&self.directory, artifact_id)? {
                transaction.execute(
                    "DELETE FROM artifact_records WHERE artifact_id = ?1 AND state = 'pending'",
                    [artifact_id_to_i64(artifact_id)?],
                )?;
                continue;
            }
            let file_name = blob_name(artifact_id);
            let file = match create_blob_file(&self.directory, &file_name) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    // A crash may have rolled back SQLite while leaving the exclusive-created blob.
                    // Preserve that file for grace-period reconciliation, and advance the sequence.
                    transaction.execute(
                        "DELETE FROM artifact_records WHERE artifact_id = ?1 AND state = 'pending'",
                        [artifact_id_to_i64(artifact_id)?],
                    )?;
                    continue;
                }
                Err(error) => return Err(ArtifactStoreError::Io(error)),
            };
            if let Err(error) = lock_exclusive(&file) {
                drop(file);
                unlink_blob_file(&self.directory, &file_name)?;
                return Err(ArtifactStoreError::Io(error));
            }
            transaction
                .commit()
                .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))?;
            return Ok((artifact_id, expires_at_ms, file));
        }
        Err(ArtifactStoreError::ArtifactIdAllocationExhausted)
    }

    fn finalize(
        &mut self,
        artifact_id: u64,
        size_bytes: u64,
        digest: &[u8; 32],
    ) -> Result<(), ArtifactStoreError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE artifact_records SET state = 'ready', size_bytes = ?2, reserved_bytes = 0, sha256 = ?3 \
             WHERE artifact_id = ?1 AND state = 'pending' AND reserved_bytes >= ?2 \
             AND NOT EXISTS(SELECT 1 FROM artifact_staging WHERE artifact_id = ?1)",
            rusqlite::params![
                artifact_id_to_i64(artifact_id)?,
                i64::try_from(size_bytes).map_err(|_| ArtifactStoreError::ArtifactTooLarge)?,
                digest.as_slice()
            ],
        )?;
        if changed != 1 {
            return Err(ArtifactStoreError::InvalidDatabaseState);
        }
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))
    }

    fn remove_pending(
        &mut self,
        artifact_id: u64,
        remove_file: bool,
    ) -> Result<(), ArtifactStoreError> {
        if remove_file {
            staging::remove_registered_directory(
                &self.database.connection,
                &self.directory,
                &self.directory_path,
                artifact_id,
            )?;
            self.remove_blob(artifact_id)?;
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM artifact_records WHERE artifact_id = ?1 AND state = 'pending'",
            [artifact_id_to_i64(artifact_id)?],
        )?;
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))?;
        Ok(())
    }

    fn load_record(&self, artifact_id: u64) -> Result<ArtifactRecord, ArtifactStoreError> {
        let artifact_id = artifact_id_to_i64(artifact_id)?;
        self.database
            .connection
            .query_row(
                "SELECT artifact_id, kind, state, size_bytes, reserved_bytes, created_at_ms, expires_at_ms, sha256 \
                 FROM artifact_records WHERE artifact_id = ?1",
                [artifact_id],
                ArtifactRecord::from_row,
            )
            .optional()?
            .ok_or(ArtifactStoreError::ArtifactNotFound(artifact_id as u64))
    }

    fn remove_blob(&self, artifact_id: u64) -> Result<(), ArtifactStoreError> {
        let name = blob_name(artifact_id);
        unlink_blob_file(&self.directory, &name).map_err(ArtifactStoreError::Io)?;
        self.directory.sync_all().map_err(ArtifactStoreError::Io)
    }

    fn lock_blob_exclusive(
        &self,
        artifact_id: u64,
    ) -> Result<Option<fs::File>, ArtifactStoreError> {
        let name = blob_name(artifact_id);
        let file = match open_blob_file(&self.directory, &name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if is_symlink_loop_error(&error) => return Ok(None),
            Err(error) => return Err(ArtifactStoreError::Io(error)),
        };
        lock_exclusive(&file)?;
        Ok(Some(file))
    }

    fn finish_deleting(&mut self, artifact_id: u64) -> Result<(), ArtifactStoreError> {
        let lock = self.lock_blob_exclusive(artifact_id)?;
        self.remove_blob(artifact_id)?;
        self.delete_record(artifact_id, "deleting")?;
        drop(lock);
        Ok(())
    }

    fn cleanup_deleting(
        &mut self,
        report: &mut ArtifactCleanupReport,
    ) -> Result<(), ArtifactStoreError> {
        let ids = self.select_ids("deleting", None)?;
        for artifact_id in ids {
            self.finish_deleting(artifact_id)?;
            report.deleting_records_reconciled =
                report.deleting_records_reconciled.saturating_add(1);
        }
        Ok(())
    }

    fn cleanup_expired_temporary(
        &mut self,
        now_ms: i64,
        report: &mut ArtifactCleanupReport,
    ) -> Result<(), ArtifactStoreError> {
        let mut statement = self.database.connection.prepare(
            "SELECT artifact_id FROM artifact_records WHERE state = 'ready' AND kind = 'temporary' \
             AND expires_at_ms <= ?1 ORDER BY expires_at_ms, artifact_id LIMIT ?2",
        )?;
        let rows = statement.query_map(
            rusqlite::params![now_ms, self.policy.cleanup_batch_size as i64],
            |row| row.get::<_, i64>(0),
        )?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(nonnegative_u64(row?)?);
        }
        drop(statement);
        for artifact_id in ids {
            if !self.mark_deleting(artifact_id)? {
                continue;
            }
            self.finish_deleting(artifact_id)?;
            report.temporary_artifacts_removed =
                report.temporary_artifacts_removed.saturating_add(1);
        }
        Ok(())
    }

    fn cleanup_stale_pending(
        &mut self,
        now_ms: i64,
        report: &mut ArtifactCleanupReport,
    ) -> Result<(), ArtifactStoreError> {
        let grace_ms = i64::try_from(self.policy.orphan_grace_ms)
            .map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
        let Some(cutoff_ms) = now_ms.checked_sub(grace_ms) else {
            return Ok(());
        };
        let ids = self.select_ids("pending", Some(cutoff_ms))?;
        for artifact_id in ids {
            if self.remove_stale_pending(artifact_id, cutoff_ms)? {
                report.pending_writes_removed = report.pending_writes_removed.saturating_add(1);
            }
        }
        Ok(())
    }

    fn cleanup_orphan_files(
        &mut self,
        now_ms: i64,
        report: &mut ArtifactCleanupReport,
    ) -> Result<(), ArtifactStoreError> {
        let known_ids = self.known_artifact_ids()?;
        let orphan_ids = find_stale_orphan_ids(
            &self.directory,
            now_ms,
            self.policy.orphan_grace_ms,
            self.policy.cleanup_batch_size,
            &known_ids,
            &mut report.scan_truncated,
        )?;
        for artifact_id in orphan_ids {
            let file = match self.lock_blob_exclusive(artifact_id)? {
                Some(file) => file,
                None => {
                    report.more_work_pending = true;
                    continue;
                }
            };
            let record_exists: bool = self.database.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifact_records WHERE artifact_id = ?1)",
                [artifact_id_to_i64(artifact_id)?],
                |row| row.get(0),
            )?;
            if record_exists {
                drop(file);
                continue;
            }
            if !staging::directory_is_old(
                &self.directory,
                artifact_id,
                now_ms,
                self.policy.orphan_grace_ms,
            )? {
                continue;
            }
            if staging::remove_directory(&self.directory, &self.directory_path, artifact_id)? {
                report.orphan_staging_directories_removed += 1;
            }
            self.remove_blob(artifact_id)?;
            drop(file);
            report.orphan_files_removed = report.orphan_files_removed.saturating_add(1);
        }
        Ok(())
    }

    fn select_ids(
        &self,
        state: &str,
        created_before_ms: Option<i64>,
    ) -> Result<Vec<u64>, ArtifactStoreError> {
        let mut statement = self.database.connection.prepare(
            "SELECT artifact_id FROM artifact_records WHERE state = ?1 \
             AND (?2 IS NULL OR created_at_ms <= ?2) ORDER BY artifact_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            rusqlite::params![
                state,
                created_before_ms,
                self.policy.cleanup_batch_size as i64
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(nonnegative_u64(row?)?);
        }
        Ok(ids)
    }

    fn mark_deleting(&mut self, artifact_id: u64) -> Result<bool, ArtifactStoreError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE artifact_records SET state = 'deleting' WHERE artifact_id = ?1 AND state = 'ready'",
            [artifact_id_to_i64(artifact_id)?],
        )?;
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))?;
        Ok(changed == 1)
    }

    fn delete_record(
        &mut self,
        artifact_id: u64,
        expected_state: &str,
    ) -> Result<(), ArtifactStoreError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM artifact_records WHERE artifact_id = ?1 AND state = ?2",
            rusqlite::params![artifact_id_to_i64(artifact_id)?, expected_state],
        )?;
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))?;
        Ok(())
    }

    fn remove_stale_pending(
        &mut self,
        artifact_id: u64,
        cutoff_ms: i64,
    ) -> Result<bool, ArtifactStoreError> {
        let name = blob_name(artifact_id);
        let file = match open_blob_file(&self.directory, &name) {
            Ok(file) => {
                if !try_lock_exclusive(&file)? {
                    return Ok(false);
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
        let still_stale: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM artifact_records \
             WHERE artifact_id = ?1 AND state = 'pending' AND created_at_ms <= ?2)",
            rusqlite::params![artifact_id_to_i64(artifact_id)?, cutoff_ms],
            |row| row.get(0),
        )?;
        if !still_stale {
            return Ok(false);
        }
        staging::remove_registered_directory(&transaction, directory, directory_path, artifact_id)?;
        if file.is_some() {
            unlink_blob_file(directory, &name).map_err(ArtifactStoreError::Io)?;
        }
        directory.sync_all().map_err(ArtifactStoreError::Io)?;
        transaction.execute(
            "DELETE FROM artifact_records WHERE artifact_id = ?1 AND state = 'pending' AND created_at_ms <= ?2",
            rusqlite::params![artifact_id_to_i64(artifact_id)?, cutoff_ms],
        )?;
        transaction
            .commit()
            .map_err(|_| ArtifactStoreError::CommitOutcomeUncertain(artifact_id))?;
        drop(file);
        Ok(true)
    }

    fn known_artifact_ids(&self) -> Result<std::collections::HashSet<u64>, ArtifactStoreError> {
        catalog_artifact_ids(&self.database.connection)
    }
}

fn catalog_artifact_ids(
    connection: &rusqlite::Connection,
) -> Result<std::collections::HashSet<u64>, ArtifactStoreError> {
    let mut statement = connection.prepare("SELECT artifact_id FROM artifact_records LIMIT ?1")?;
    let rows = statement.query_map([MAX_ARTIFACT_COUNT + 1], |row| row.get::<_, i64>(0))?;
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        ids.insert(nonnegative_u64(row?)?);
        if ids.len() > MAX_ARTIFACT_COUNT as usize {
            return Err(ArtifactStoreError::ReconciliationRequired);
        }
    }
    Ok(ids)
}

fn catalog_pending_ids(
    connection: &rusqlite::Connection,
) -> Result<std::collections::HashSet<u64>, ArtifactStoreError> {
    let mut statement = connection
        .prepare("SELECT artifact_id FROM artifact_records WHERE state = 'pending' LIMIT ?1")?;
    let rows = statement.query_map([MAX_ARTIFACT_COUNT + 1], |row| row.get::<_, i64>(0))?;
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        ids.insert(nonnegative_u64(row?)?);
        if ids.len() > MAX_ARTIFACT_COUNT as usize {
            return Err(ArtifactStoreError::ReconciliationRequired);
        }
    }
    Ok(ids)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn managed_orphan_bytes(
    directory: &fs::File,
    known_ids: &std::collections::HashSet<u64>,
    pending_ids: &std::collections::HashSet<u64>,
) -> Result<u64, ArtifactStoreError> {
    use rustix::{
        fd::AsFd,
        fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat},
    };

    let descriptor = openat(
        directory.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
    let mut stream =
        Dir::new(descriptor).map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    while let Some(entry) = stream.read() {
        let entry = entry.map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        entries += 1;
        if entries > MAX_ARTIFACT_DIRECTORY_SCAN_ENTRIES {
            return Err(ArtifactStoreError::ReconciliationRequired);
        }
        let Ok(name_str) = std::str::from_utf8(name.to_bytes()) else {
            continue;
        };
        if let Some(id) = staging::parse_name(name_str) {
            if !pending_ids.contains(&id) {
                bytes = bytes
                    .checked_add(staging::tree_bytes(directory, id)?)
                    .ok_or(ArtifactStoreError::InvalidDatabaseState)?;
            }
            continue;
        }
        let Some(id) = parse_blob_name(name_str) else {
            continue;
        };
        if known_ids.contains(&id) {
            continue;
        }
        let stat = match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(error) if error == rustix::io::Errno::NOENT => continue,
            Err(error) => return Err(ArtifactStoreError::Io(io::Error::from(error))),
        };
        if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
            bytes = bytes
                .checked_add(nonnegative_u64(stat.st_size)?)
                .ok_or(ArtifactStoreError::InvalidDatabaseState)?;
        }
    }
    Ok(bytes)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn managed_orphan_bytes(
    _directory: &fs::File,
    _known_ids: &std::collections::HashSet<u64>,
    _pending_ids: &std::collections::HashSet<u64>,
) -> Result<u64, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}

struct ArtifactRecord {
    id: i64,
    kind: String,
    state: String,
    size_bytes: i64,
    reserved_bytes: i64,
    created_at_ms: i64,
    expires_at_ms: Option<i64>,
    sha256: Option<Vec<u8>>,
}

impl ArtifactRecord {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            kind: row.get(1)?,
            state: row.get(2)?,
            size_bytes: row.get(3)?,
            reserved_bytes: row.get(4)?,
            created_at_ms: row.get(5)?,
            expires_at_ms: row.get(6)?,
            sha256: row.get(7)?,
        })
    }

    fn metadata(&self) -> Result<ArtifactMetadata, ArtifactStoreError> {
        if self.id <= 0 || self.size_bytes < 0 || self.reserved_bytes < 0 || self.created_at_ms < 0
        {
            return Err(ArtifactStoreError::InvalidDatabaseState);
        }
        Ok(ArtifactMetadata {
            id: self.id as u64,
            kind: ArtifactKind::from_db_value(&self.kind)?,
            size_bytes: self.size_bytes as u64,
            created_at_ms: self.created_at_ms,
            expires_at_ms: self.expires_at_ms,
        })
    }
}

fn duration_millis(duration: Duration) -> Result<u64, ArtifactStoreError> {
    let millis = duration.as_millis();
    if millis == 0 || millis > i64::MAX as u128 {
        return Err(ArtifactStoreError::InvalidPolicy);
    }
    Ok(millis as u64)
}

fn validate_timestamp(now_ms: i64) -> Result<(), ArtifactStoreError> {
    if now_ms < 0 {
        return Err(ArtifactStoreError::InvalidTimestamp);
    }
    Ok(())
}

fn nonnegative_u64(value: i64) -> Result<u64, ArtifactStoreError> {
    u64::try_from(value).map_err(|_| ArtifactStoreError::InvalidDatabaseState)
}

fn artifact_id_to_i64(artifact_id: u64) -> Result<i64, ArtifactStoreError> {
    i64::try_from(artifact_id)
        .ok()
        .filter(|id| *id > 0)
        .ok_or(ArtifactStoreError::ArtifactNotFound(artifact_id))
}

fn blob_name(artifact_id: u64) -> String {
    format!("artifact-{artifact_id}.blob")
}

fn copy_bounded<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    maximum_bytes: u64,
) -> Result<(u64, [u8; 32]), ArtifactStoreError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    let mut total_bytes = 0_u64;
    loop {
        let remaining = maximum_bytes.saturating_sub(total_bytes);
        let read_capacity = if remaining == 0 {
            1
        } else {
            usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
                .map_err(|_| ArtifactStoreError::ArtifactTooLarge)?
        };
        let count = match source.read(&mut buffer[..read_capacity]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ArtifactStoreError::Io(error)),
        };
        if count == 0 {
            break;
        }
        if count as u64 > remaining {
            return Err(ArtifactStoreError::ArtifactTooLarge);
        }
        destination.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        total_bytes = total_bytes
            .checked_add(count as u64)
            .ok_or(ArtifactStoreError::ArtifactTooLarge)?;
    }
    let digest = hasher.finalize();
    let mut digest_bytes = [0_u8; 32];
    digest_bytes.copy_from_slice(&digest);
    Ok((total_bytes, digest_bytes))
}

fn map_content_open_error(error: io::Error) -> ArtifactStoreError {
    if error.kind() == io::ErrorKind::NotFound {
        return ArtifactStoreError::ArtifactContentMissing;
    }
    ArtifactStoreError::Io(error)
}

fn is_symlink_loop_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lock_exclusive(file: &fs::File) -> Result<(), io::Error> {
    use rustix::fs::{FlockOperation, flock};

    flock(file, FlockOperation::LockExclusive).map_err(io::Error::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn lock_exclusive(_file: &fs::File) -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "artifact store is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn try_lock_exclusive(file: &fs::File) -> Result<bool, io::Error> {
    use rustix::fs::{FlockOperation, flock};

    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) => {
            let error = io::Error::from(error);
            if error.kind() == io::ErrorKind::WouldBlock {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn try_lock_exclusive(_file: &fs::File) -> Result<bool, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "artifact store is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lock_shared(file: &fs::File) -> Result<(), ArtifactStoreError> {
    use rustix::fs::{FlockOperation, flock};

    flock(file, FlockOperation::LockShared)
        .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn lock_shared(_file: &fs::File) -> Result<(), ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_private_directory(path: &Path) -> Result<fs::File, ArtifactStoreError> {
    use rustix::{
        fd::AsFd,
        fs::{Mode, OFlags, openat},
    };
    use std::os::unix::fs::MetadataExt;

    if !path.is_absolute() {
        return Err(ArtifactStoreError::UserDatabase(
            UserDatabaseError::UnsafePath(path.to_owned()),
        ));
    }
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.is_dir() || path_metadata.mode() & 0o077 != 0 {
        return Err(ArtifactStoreError::UserDatabase(
            UserDatabaseError::UnsafePath(path.to_owned()),
        ));
    }

    let mut current = fs::File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let descriptor = openat(
                    current.as_fd(),
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
                current = fs::File::from(descriptor);
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(ArtifactStoreError::UserDatabase(
                    UserDatabaseError::UnsafePath(path.to_owned()),
                ));
            }
        }
    }

    let opened_metadata = current.metadata()?;
    if !opened_metadata.is_dir()
        || opened_metadata.dev() != path_metadata.dev()
        || opened_metadata.ino() != path_metadata.ino()
    {
        return Err(ArtifactStoreError::UserDatabase(
            UserDatabaseError::UnsafePath(path.to_owned()),
        ));
    }
    Ok(current)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_private_directory(_path: &Path) -> Result<fs::File, ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_blob_file(directory: &fs::File, name: &str) -> Result<fs::File, io::Error> {
    use rustix::{
        fd::AsFd,
        fs::{Mode, OFlags, openat},
    };

    let descriptor = openat(
        directory.as_fd(),
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(io::Error::from)?;
    Ok(fs::File::from(descriptor))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_blob_file(_directory: &fs::File, _name: &str) -> Result<fs::File, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "artifact store is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_blob_file(directory: &fs::File, name: &str) -> Result<fs::File, io::Error> {
    use rustix::{
        fd::AsFd,
        fs::{Mode, OFlags, openat},
    };

    let descriptor = openat(
        directory.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    Ok(fs::File::from(descriptor))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_blob_file(_directory: &fs::File, _name: &str) -> Result<fs::File, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "artifact store is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unlink_blob_file(directory: &fs::File, name: &str) -> Result<(), io::Error> {
    use rustix::{
        fd::AsFd,
        fs::{AtFlags, unlinkat},
    };

    match unlinkat(directory.as_fd(), name, AtFlags::empty()) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unlink_blob_file(_directory: &fs::File, _name: &str) -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "artifact store is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_private_artifact_file(
    file: &fs::File,
    expected_size: u64,
) -> Result<(), ArtifactStoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() != expected_size
    {
        return Err(ArtifactStoreError::ArtifactIntegrityMismatch);
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_private_artifact_file(
    _file: &fs::File,
    _expected_size: u64,
) -> Result<(), ArtifactStoreError> {
    Err(ArtifactStoreError::UnsupportedPlatform)
}

fn find_stale_orphan_ids(
    directory: &fs::File,
    now_ms: i64,
    grace_ms: u64,
    batch_size: usize,
    known_ids: &std::collections::HashSet<u64>,
    scan_truncated: &mut bool,
) -> Result<Vec<u64>, ArtifactStoreError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::{
            fd::AsFd,
            fs::{Dir, Mode, OFlags, openat},
        };
        let stream_fd = openat(
            directory.as_fd(),
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        let mut stream =
            Dir::new(stream_fd).map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
        let mut orphan_ids = Vec::new();
        let mut entries_seen = 0_usize;
        while let Some(entry_result) = stream.read() {
            let entry =
                entry_result.map_err(|error| ArtifactStoreError::Io(io::Error::from(error)))?;
            let name = entry.file_name();
            let name_bytes = name.to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            entries_seen = entries_seen.saturating_add(1);
            if entries_seen > MAX_ARTIFACT_DIRECTORY_SCAN_ENTRIES {
                *scan_truncated = true;
                break;
            }
            let Ok(name) = std::str::from_utf8(name_bytes) else {
                continue;
            };
            let Some(artifact_id) = parse_blob_name(name) else {
                continue;
            };
            if known_ids.contains(&artifact_id) {
                continue;
            }
            let Ok(file) = open_blob_file(directory, name) else {
                continue;
            };
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                continue;
            }
            let modified = metadata.modified()?;
            let Ok(modified_duration) = modified.duration_since(std::time::UNIX_EPOCH) else {
                continue;
            };
            let modified_ms = i64::try_from(modified_duration.as_millis())
                .map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
            let grace_ms =
                i64::try_from(grace_ms).map_err(|_| ArtifactStoreError::InvalidTimestamp)?;
            if now_ms.saturating_sub(modified_ms) >= grace_ms {
                orphan_ids.push(artifact_id);
                if orphan_ids.len() >= batch_size {
                    *scan_truncated = true;
                    break;
                }
            }
        }
        Ok(orphan_ids)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (
            directory,
            now_ms,
            grace_ms,
            batch_size,
            known_ids,
            scan_truncated,
        );
        Err(ArtifactStoreError::UnsupportedPlatform)
    }
}

fn parse_blob_name(name: &str) -> Option<u64> {
    let numeric_id = name.strip_prefix("artifact-")?.strip_suffix(".blob")?;
    let artifact_id = numeric_id.parse::<u64>().ok()?;
    if artifact_id == 0 || artifact_id > i64::MAX as u64 || blob_name(artifact_id) != name {
        return None;
    }
    Some(artifact_id)
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactKind, ArtifactPolicy, ArtifactStore, ArtifactStoreError, DEFAULT_ARTIFACT_RETENTION,
    };
    use crate::SCHEMA_VERSION;
    use rusqlite::Connection;
    use std::{
        fs,
        io::{Cursor, Read, Write},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;

            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "carapana-artifacts-{}-{nonce}-{id}",
                    std::process::id()
                ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn database_path(&self) -> PathBuf {
            self.0.join("sessions.sqlite3")
        }

        fn artifact_path(&self, artifact_id: u64) -> PathBuf {
            self.0
                .join("artifacts")
                .join(format!("artifact-{artifact_id}.blob"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn policy(
        total_bytes: u64,
        artifact_bytes: u64,
        retention: Duration,
        orphan_grace: Duration,
    ) -> ArtifactPolicy {
        ArtifactPolicy::new(total_bytes, artifact_bytes, retention, orphan_grace).unwrap()
    }

    #[test]
    fn should_stream_read_list_and_explicitly_delete_private_artifacts() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDirectory::new();
        let policy = policy(64, 32, Duration::from_secs(100), Duration::from_secs(100));
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let payload = b"bounded local artifact";
        let metadata = store
            .write(
                ArtifactKind::Durable,
                32,
                Cursor::new(payload.as_slice()),
                1_000,
            )
            .unwrap();

        assert_eq!(metadata.size_bytes, payload.len() as u64);
        assert_eq!(metadata.expires_at_ms, None);
        assert_eq!(store.list(None, 10, 1_000).unwrap(), vec![metadata.clone()]);
        let mut copied = Vec::new();
        store.copy_to(metadata.id, 1_001, &mut copied).unwrap();
        assert_eq!(copied, payload);
        assert_eq!(
            fs::metadata(root.0.join("artifacts"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.artifact_path(metadata.id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let usage = store.usage().unwrap();
        assert_eq!(usage.used_bytes, payload.len() as u64);
        assert_eq!(usage.reserved_bytes, 0);

        store.delete(metadata.id).unwrap();
        assert!(store.list(None, 10, 1_000).unwrap().is_empty());
        assert_eq!(store.usage().unwrap().used_bytes, 0);
    }

    #[test]
    fn should_reject_over_limit_streams_and_release_reserved_capacity() {
        let root = TestDirectory::new();
        let mut store = ArtifactStore::open_with_policy(
            root.database_path(),
            policy(4, 4, Duration::from_secs(100), Duration::from_secs(100)),
        )
        .unwrap();

        assert!(matches!(
            store.write(ArtifactKind::Durable, 4, Cursor::new(b"12345"), 1_000),
            Err(ArtifactStoreError::ArtifactTooLarge)
        ));
        let usage = store.usage().unwrap();
        assert_eq!(usage.used_bytes, 0);
        assert_eq!(usage.reserved_bytes, 0);
        assert_eq!(usage.artifact_count, 0);
        assert_eq!(fs::read_dir(root.0.join("artifacts")).unwrap().count(), 0);
    }

    #[test]
    fn should_remove_partial_files_and_reservations_when_the_source_fails() {
        let root = TestDirectory::new();
        let mut store = ArtifactStore::open_with_policy(
            root.database_path(),
            policy(16, 8, Duration::from_secs(100), Duration::from_secs(100)),
        )
        .unwrap();

        assert!(matches!(
            store.write(ArtifactKind::Durable, 8, FailingReader, 1_000),
            Err(ArtifactStoreError::Io(_))
        ));
        let usage = store.usage().unwrap();
        assert_eq!(usage.used_bytes, 0);
        assert_eq!(usage.reserved_bytes, 0);
        assert_eq!(usage.artifact_count, 0);
        assert_eq!(fs::read_dir(root.0.join("artifacts")).unwrap().count(), 0);
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic source failure"))
        }
    }

    #[test]
    fn should_reserve_global_quota_before_reading_the_source() {
        let root = TestDirectory::new();
        let policy = policy(8, 8, Duration::from_secs(100), Duration::from_secs(100));
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let mut source = ReservationProbeReader {
            database_path: root.database_path(),
            policy,
            payload: Cursor::new(b"first".to_vec()),
            attempted: false,
            second_write_rejected: false,
        };

        store
            .write(ArtifactKind::Durable, 8, &mut source, 1_000)
            .unwrap();
        assert!(source.second_write_rejected);
        assert_eq!(store.usage().unwrap().used_bytes, 5);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_not_clean_an_expired_reservation_while_its_writer_holds_the_file_lock() {
        let root = TestDirectory::new();
        let policy = policy(
            16,
            8,
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let mut source = StaleCleanupProbeReader {
            database_path: root.database_path(),
            policy,
            payload: Cursor::new(b"active".to_vec()),
            attempted: false,
            pending_writes_removed: None,
            reserved_bytes: None,
        };

        store
            .write(ArtifactKind::Durable, 8, &mut source, 1_000)
            .unwrap();

        assert_eq!(source.pending_writes_removed, Some(0));
        assert_eq!(source.reserved_bytes, Some(8));
        assert_eq!(store.usage().unwrap().used_bytes, 6);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    struct ReservationProbeReader {
        database_path: PathBuf,
        policy: ArtifactPolicy,
        payload: Cursor<Vec<u8>>,
        attempted: bool,
        second_write_rejected: bool,
    }

    impl Read for ReservationProbeReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.attempted {
                self.attempted = true;
                let mut second_store =
                    ArtifactStore::open_with_policy(&self.database_path, self.policy)
                        .map_err(std::io::Error::other)?;
                self.second_write_rejected = matches!(
                    second_store.write(ArtifactKind::Durable, 1, Cursor::new(b"x"), 1_000,),
                    Err(ArtifactStoreError::QuotaExceeded { .. })
                );
            }
            self.payload.read(buffer)
        }
    }

    struct StaleCleanupProbeReader {
        database_path: PathBuf,
        policy: ArtifactPolicy,
        payload: Cursor<Vec<u8>>,
        attempted: bool,
        pending_writes_removed: Option<u64>,
        reserved_bytes: Option<u64>,
    }

    impl Read for StaleCleanupProbeReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.attempted {
                self.attempted = true;
                let mut concurrent_store =
                    ArtifactStore::open_with_policy(&self.database_path, self.policy)
                        .map_err(std::io::Error::other)?;
                let report = concurrent_store
                    .cleanup(1_200)
                    .map_err(std::io::Error::other)?;
                self.pending_writes_removed = Some(report.pending_writes_removed);
                self.reserved_bytes = Some(
                    concurrent_store
                        .usage()
                        .map_err(std::io::Error::other)?
                        .reserved_bytes,
                );
            }
            self.payload.read(buffer)
        }
    }

    #[test]
    fn should_expire_temporary_artifacts_without_deleting_durable_work() {
        let root = TestDirectory::new();
        let retention = Duration::from_millis(100);
        let mut store = ArtifactStore::open_with_policy(
            root.database_path(),
            policy(64, 16, retention, Duration::from_millis(100)),
        )
        .unwrap();
        let durable = store
            .write(ArtifactKind::Durable, 8, Cursor::new(b"keep"), 1_000)
            .unwrap();
        let temporary = store
            .write(ArtifactKind::Temporary, 8, Cursor::new(b"expire"), 1_000)
            .unwrap();
        assert_eq!(temporary.expires_at_ms, Some(1_100));
        assert_eq!(store.list(None, 10, 1_100).unwrap(), vec![durable.clone()]);

        let report = store.cleanup(1_100).unwrap();
        assert_eq!(report.temporary_artifacts_removed, 1);
        assert_eq!(store.list(None, 10, 1_100).unwrap(), vec![durable.clone()]);
        assert!(matches!(
            store.copy_to(temporary.id, 1_100, &mut Vec::new()),
            Err(ArtifactStoreError::ArtifactNotFound(_))
        ));
        let mut copied = Vec::new();
        store.copy_to(durable.id, 1_101, &mut copied).unwrap();
        assert_eq!(copied, b"keep");
    }

    #[test]
    fn should_report_when_bounded_cleanup_has_more_work() {
        let root = TestDirectory::new();
        let policy = policy(
            64,
            16,
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .with_cleanup_batch_size(1)
        .unwrap();
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        store
            .write(ArtifactKind::Temporary, 8, Cursor::new(b"one"), 1_000)
            .unwrap();
        store
            .write(ArtifactKind::Temporary, 8, Cursor::new(b"two"), 1_000)
            .unwrap();

        let first = store.cleanup(1_100).unwrap();
        assert_eq!(first.temporary_artifacts_removed, 1);
        assert!(first.more_work_pending);
        let second = store.cleanup(1_100).unwrap();
        assert_eq!(second.temporary_artifacts_removed, 1);
        assert!(!second.more_work_pending);
    }

    #[test]
    fn should_remove_only_managed_orphan_files_past_the_grace_period() {
        use std::fs::{FileTimes, OpenOptions};
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let mut store = ArtifactStore::open_with_policy(
            root.database_path(),
            policy(
                64,
                16,
                Duration::from_millis(100),
                Duration::from_millis(100),
            ),
        )
        .unwrap();
        let stale_path = root.artifact_path(9_999);
        let stale_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stale_path)
            .unwrap();
        stale_file
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        drop(stale_file);

        let recent_path = root.artifact_path(10_000);
        let recent_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&recent_path)
            .unwrap();
        recent_file
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(950)))
            .unwrap();
        drop(recent_file);

        let noncanonical_path = root.0.join("artifacts/artifact-00009999.blob");
        let noncanonical_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&noncanonical_path)
            .unwrap();
        noncanonical_file
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        drop(noncanonical_file);

        let report = store.cleanup(1_000).unwrap();
        assert_eq!(report.orphan_files_removed, 1);
        assert!(!stale_path.exists());
        assert!(recent_path.exists());
        assert!(noncanonical_path.exists());
    }

    #[test]
    fn should_reject_alternate_catalog_names_before_sharing_an_artifact_directory() {
        let root = TestDirectory::new();
        let mut primary = ArtifactStore::open(root.database_path()).unwrap();
        let artifact = primary
            .write(ArtifactKind::Durable, 16, Cursor::new(b"keep"), 1_000)
            .unwrap();
        let other_database = root.0.join("other.sqlite3");
        assert!(matches!(
            ArtifactStore::open(&other_database),
            Err(ArtifactStoreError::UserDatabase(
                crate::UserDatabaseError::UnsafePath(_)
            ))
        ));
        assert!(!other_database.exists());
        let mut bytes = Vec::new();
        primary.copy_to(artifact.id, 1_001, &mut bytes).unwrap();
        assert_eq!(bytes, b"keep");
    }

    #[test]
    fn should_charge_unexpired_orphan_payloads_against_admission_quota() {
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let policy = policy(16, 16, Duration::from_secs(100), Duration::from_secs(100));
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let mut orphan = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.artifact_path(90))
            .unwrap();
        orphan.write_all(b"orphaned").unwrap();
        drop(orphan);

        assert!(matches!(
            store.write(ArtifactKind::Durable, 16, Cursor::new(b"new"), 1_000),
            Err(ArtifactStoreError::QuotaExceeded { used_bytes: 8, .. })
        ));
        let usage = store.usage().unwrap();
        assert_eq!(usage.orphan_bytes, 8);
        assert_eq!(usage.reserved_bytes, 0);
        let metadata = store
            .write(ArtifactKind::Durable, 8, Cursor::new(b"new"), 1_000)
            .unwrap();
        assert_eq!(metadata.size_bytes, 3);
        assert_eq!(fs::read(root.artifact_path(90)).unwrap(), b"orphaned");
    }

    #[test]
    fn should_bound_id_collision_retries_without_touching_existing_blobs() {
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let mut store = ArtifactStore::open(root.database_path()).unwrap();
        for id in 1..=super::MAX_ARTIFACT_ID_ALLOCATION_ATTEMPTS as u64 {
            let _file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.artifact_path(id))
                .unwrap();
        }
        assert!(matches!(
            store.write(ArtifactKind::Durable, 8, Cursor::new(b"new"), 1_000),
            Err(ArtifactStoreError::ArtifactIdAllocationExhausted)
        ));
        assert_eq!(store.usage().unwrap().artifact_count, 0);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert_eq!(
            fs::read_dir(root.0.join("artifacts")).unwrap().count(),
            super::MAX_ARTIFACT_ID_ALLOCATION_ATTEMPTS
        );
    }

    #[test]
    fn should_skip_uncommitted_blob_ids_without_overwriting_or_deleting_them() {
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let mut store = ArtifactStore::open(root.database_path()).unwrap();
        let orphan_path = root.artifact_path(1);
        let mut orphan = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&orphan_path)
            .unwrap();
        orphan.write_all(b"uncommitted interrupted write").unwrap();
        drop(orphan);

        let metadata = store
            .write(
                ArtifactKind::Durable,
                16,
                Cursor::new(b"new artifact"),
                1_000,
            )
            .unwrap();
        assert_eq!(metadata.id, 2);
        assert_eq!(
            fs::read(&orphan_path).unwrap(),
            b"uncommitted interrupted write"
        );
        let mut copied = Vec::new();
        store.copy_to(metadata.id, 1_001, &mut copied).unwrap();
        assert_eq!(copied, b"new artifact");
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert_eq!(store.usage().unwrap().artifact_count, 1);
    }

    #[test]
    fn should_recover_stale_pending_records_and_their_files_after_a_crash() {
        use std::fs::{FileTimes, OpenOptions};
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let policy = policy(
            64,
            16,
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        let store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        drop(store);

        let stale_path = root.artifact_path(7);
        let mut stale_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stale_path)
            .unwrap();
        stale_file.write_all(b"partial").unwrap();
        stale_file
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(1)))
            .unwrap();
        drop(stale_file);

        let connection = Connection::open(root.database_path()).unwrap();
        connection
            .execute(
                "INSERT INTO artifact_records \
                 (artifact_id, kind, state, size_bytes, reserved_bytes, created_at_ms, expires_at_ms, sha256) \
                 VALUES (7, 'durable', 'pending', 0, 16, 1_000, NULL, NULL)",
                [],
            )
            .unwrap();
        drop(connection);

        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let report = store.cleanup(1_200).unwrap();
        assert_eq!(report.pending_writes_removed, 1);
        assert!(!stale_path.exists());
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert_eq!(store.usage().unwrap().artifact_count, 0);
    }

    #[test]
    fn should_reconcile_deleting_records_without_losing_quota_accounting() {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let root = TestDirectory::new();
        let policy = policy(
            64,
            16,
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        let artifact = store
            .write(ArtifactKind::Durable, 16, Cursor::new(b"reconcile"), 1_000)
            .unwrap();
        let blob_path = root.artifact_path(artifact.id);
        drop(store);

        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&blob_path)
            .unwrap();
        file.write_all(b"reconcile").unwrap();
        drop(file);
        let connection = Connection::open(root.database_path()).unwrap();
        connection
            .execute(
                "UPDATE artifact_records SET state = 'deleting' WHERE artifact_id = ?1",
                [artifact.id as i64],
            )
            .unwrap();
        drop(connection);

        let mut store = ArtifactStore::open_with_policy(root.database_path(), policy).unwrap();
        assert_eq!(store.usage().unwrap().used_bytes, artifact.size_bytes);
        let report = store.cleanup(1_001).unwrap();
        assert_eq!(report.deleting_records_reconciled, 1);
        assert!(!blob_path.exists());
        assert_eq!(store.usage().unwrap().used_bytes, 0);
        assert_eq!(store.usage().unwrap().artifact_count, 0);
    }

    #[test]
    fn should_keep_durable_metadata_when_content_is_missing_or_corrupt() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDirectory::new();
        let mut store = ArtifactStore::open(root.database_path()).unwrap();
        let metadata = store
            .write(ArtifactKind::Durable, 16, Cursor::new(b"original"), 1_000)
            .unwrap();
        let path = root.artifact_path(metadata.id);
        fs::write(&path, b"corrupt!").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            store.copy_to(metadata.id, 1_001, &mut Vec::new()),
            Err(ArtifactStoreError::ArtifactIntegrityMismatch)
        ));

        fs::remove_file(&path).unwrap();
        assert!(matches!(
            store.copy_to(metadata.id, 1_002, &mut Vec::new()),
            Err(ArtifactStoreError::ArtifactContentMissing)
        ));
        assert_eq!(store.list(None, 10, 1_001).unwrap(), vec![metadata]);
        assert_eq!(store.usage().unwrap().used_bytes, 8);
    }

    #[test]
    fn should_not_hide_artifacts_when_the_destination_write_fails() {
        let root = TestDirectory::new();
        let mut store = ArtifactStore::open(root.database_path()).unwrap();
        let metadata = store
            .write(
                ArtifactKind::Durable,
                16,
                Cursor::new(b"destination"),
                1_000,
            )
            .unwrap();
        let mut destination = FailingWriter;

        assert!(matches!(
            store.copy_to(metadata.id, 1_001, &mut destination),
            Err(ArtifactStoreError::Io(_))
        ));
        assert_eq!(store.list(None, 10, 1_001).unwrap(), vec![metadata]);
        assert_eq!(store.usage().unwrap().used_bytes, 11);
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic destination failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_to_follow_a_symlinked_artifact_blob() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        let mut store = ArtifactStore::open(root.database_path()).unwrap();
        let metadata = store
            .write(ArtifactKind::Durable, 16, Cursor::new(b"safe"), 1_000)
            .unwrap();
        let target = root.0.join("outside.bin");
        fs::write(&target, b"external").unwrap();
        let blob = root.artifact_path(metadata.id);
        fs::remove_file(&blob).unwrap();
        symlink(&target, &blob).unwrap();

        assert!(matches!(
            store.copy_to(metadata.id, 1_001, &mut Vec::new()),
            Err(ArtifactStoreError::Io(_))
        ));
        assert_eq!(fs::read(target).unwrap(), b"external");
    }

    #[test]
    fn should_migrate_version_five_database_to_the_artifact_schema() {
        let root = TestDirectory::new();
        let database_path = root.database_path();
        {
            let store = ArtifactStore::open(&database_path).unwrap();
            drop(store);
        }
        {
            let connection = Connection::open(&database_path).unwrap();
            connection
                .execute_batch(
                    "DROP INDEX artifact_records_state_expiry; DROP TABLE artifact_records;",
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 5).unwrap();
        }

        let store = ArtifactStore::open(&database_path).unwrap();
        assert_eq!(store.database.schema_version().unwrap(), SCHEMA_VERSION);
        let table_exists: bool = store
            .database
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'artifact_records')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists);
    }

    #[test]
    fn should_validate_policy_against_approved_ceilings() {
        assert!(
            ArtifactPolicy::new(0, 1, DEFAULT_ARTIFACT_RETENTION, Duration::from_secs(1)).is_err()
        );
        assert!(
            ArtifactPolicy::new(1, 2, DEFAULT_ARTIFACT_RETENTION, Duration::from_secs(1)).is_err()
        );
        assert!(
            ArtifactPolicy::new(
                super::DEFAULT_ARTIFACT_STORE_QUOTA_BYTES + 1,
                1,
                DEFAULT_ARTIFACT_RETENTION,
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
