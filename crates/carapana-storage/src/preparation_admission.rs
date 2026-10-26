//! Shared nominal preparation reservations, not RSS measurement or proof of consumer termination.
//! The general launcher must acquire this before payload allocation and release only after cleanup.

use crate::{
    Database, MigrationError, UserDatabaseError, WorkspaceDirectoryIdentity,
    user_database::{
        DATABASE_FILE, ensure_private_database_file, ensure_private_directory, user_data_directory,
        user_database_path,
    },
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use std::{error::Error, fmt, path::Path};
pub(crate) mod queue;
mod recovery;
use recovery::OwnerLock;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preparation_admission_policy (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    max_bytes INTEGER NOT NULL CHECK (max_bytes > 0),
    max_leases INTEGER NOT NULL CHECK (max_leases > 0 AND max_leases <= 1024),
    workspace_max_bytes INTEGER NOT NULL CHECK (workspace_max_bytes > 0 AND workspace_max_bytes <= max_bytes),
    workspace_max_leases INTEGER NOT NULL CHECK (workspace_max_leases > 0 AND workspace_max_leases <= max_leases)
) STRICT;
CREATE TABLE IF NOT EXISTS preparation_admission_leases (
    lease_id INTEGER PRIMARY KEY AUTOINCREMENT,
    reserved_bytes INTEGER NOT NULL CHECK (reserved_bytes > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'quarantined')),
    workspace_key BLOB CHECK (workspace_key IS NULL OR length(workspace_key) = 16),
    preparation_intent INTEGER NOT NULL DEFAULT 0 CHECK(preparation_intent IN (0, 1))
) STRICT;
CREATE INDEX IF NOT EXISTS preparation_admission_workspace ON preparation_admission_leases(workspace_key);
"#;

pub(super) const RECOVERY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preparation_admission_owners (
    lease_id INTEGER PRIMARY KEY REFERENCES preparation_admission_leases(lease_id) ON DELETE CASCADE,
    file_identity BLOB NOT NULL CHECK(length(file_identity) = 16),
    exported INTEGER NOT NULL CHECK(exported IN (0, 1)),
    cleanup_confirmed INTEGER NOT NULL CHECK(cleanup_confirmed IN (0, 1) AND (cleanup_confirmed = 0 OR exported = 1))
) STRICT;
"#;

pub(super) const MIGRATE_INTENT: &str = "ALTER TABLE preparation_admission_leases ADD COLUMN preparation_intent INTEGER NOT NULL DEFAULT 0 CHECK(preparation_intent IN (0, 1));";

pub(super) const MIGRATE_V8: &str = r#"
ALTER TABLE preparation_admission_policy ADD COLUMN workspace_max_bytes INTEGER NOT NULL DEFAULT 1;
ALTER TABLE preparation_admission_policy ADD COLUMN workspace_max_leases INTEGER NOT NULL DEFAULT 1;
UPDATE preparation_admission_policy SET workspace_max_bytes = max_bytes, workspace_max_leases = max_leases;
ALTER TABLE preparation_admission_leases ADD COLUMN workspace_key BLOB CHECK (workspace_key IS NULL OR length(workspace_key) = 16);
CREATE INDEX preparation_admission_workspace ON preparation_admission_leases(workspace_key);
"#;

/// Explicit per-user nominal capacity. No hardware-calibrated defaults are selected here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationAdmissionPolicy {
    max_bytes: u64,
    max_leases: u32,
    workspace_max_bytes: u64,
    workspace_max_leases: u32,
}
impl PreparationAdmissionPolicy {
    pub fn new(max_bytes: u64, max_leases: u32) -> Result<Self, PreparationAdmissionError> {
        Self::with_workspace_limits(max_bytes, max_leases, max_bytes, max_leases)
    }

    pub fn with_workspace_limits(
        max_bytes: u64,
        max_leases: u32,
        workspace_max_bytes: u64,
        workspace_max_leases: u32,
    ) -> Result<Self, PreparationAdmissionError> {
        if max_bytes == 0 || max_bytes > i64::MAX as u64 || max_leases == 0 || max_leases > 1024 {
            return Err(PreparationAdmissionError::InvalidPolicy);
        }
        if workspace_max_bytes == 0
            || workspace_max_bytes > max_bytes
            || workspace_max_leases == 0
            || workspace_max_leases > max_leases
        {
            return Err(PreparationAdmissionError::InvalidPolicy);
        }
        Ok(Self {
            max_bytes,
            max_leases,
            workspace_max_bytes,
            workspace_max_leases,
        })
    }
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    pub fn max_leases(&self) -> u32 {
        self.max_leases
    }
    pub fn workspace_max_bytes(&self) -> u64 {
        self.workspace_max_bytes
    }
    pub fn workspace_max_leases(&self) -> u32 {
        self.workspace_max_leases
    }
}

#[derive(Debug)]
pub enum PreparationAdmissionError {
    InvalidPolicy,
    PolicyMismatch,
    InvalidRequest,
    CapacityExceeded,
    WorkspaceCapacityExceeded,
    WorkspaceRequired,
    QueueRequired,
    QueueFull,
    QueueNotReady,
    QueueExpired,
    RecoveryIo(std::io::Error),
    InvalidDatabaseState,
    CommitOutcomeUncertain,
    UserDatabase(UserDatabaseError),
    Migration(MigrationError),
    Sqlite(rusqlite::Error),
}
impl fmt::Display for PreparationAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPolicy => "invalid explicit preparation admission policy",
            Self::PolicyMismatch => "preparation policy does not match the shared user catalog",
            Self::InvalidRequest => "invalid preparation reservation request",
            Self::CapacityExceeded => {
                "shared preparation capacity is unavailable; do not expand limits automatically"
            }
            Self::InvalidDatabaseState => "preparation admission state is inconsistent",
            Self::WorkspaceCapacityExceeded => "workspace preparation capacity is unavailable",
            Self::WorkspaceRequired => "preparation policy requires an explicit physical workspace",
            Self::QueueRequired => "preparation policy requires an explicit waiting ticket",
            Self::QueueFull => "preparation waiting capacity is unavailable",
            Self::QueueNotReady => "preparation ticket is not eligible for admission",
            Self::QueueExpired => "preparation waiting ticket expired",
            Self::RecoveryIo(_) => "preparation ownership control is unavailable or unsafe",
            Self::CommitOutcomeUncertain => {
                "preparation admission commit is uncertain; reconcile before retrying"
            }
            Self::UserDatabase(_) | Self::Migration(_) | Self::Sqlite(_) => {
                "preparation admission catalog is unavailable"
            }
        })
    }
}
impl Error for PreparationAdmissionError {}
impl From<std::io::Error> for PreparationAdmissionError {
    fn from(error: std::io::Error) -> Self {
        Self::RecoveryIo(error)
    }
}
impl From<rusqlite::Error> for PreparationAdmissionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl From<MigrationError> for PreparationAdmissionError {
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}
impl From<UserDatabaseError> for PreparationAdmissionError {
    fn from(error: UserDatabaseError) -> Self {
        Self::UserDatabase(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationAdmissionUsage {
    pub reserved_bytes: u64,
    pub held_leases: u32,
    pub quarantined_leases: u32,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct PreparationRecoveryReport {
    pub recovered_leases: u32,
    pub recovered_bytes: u64,
    pub busy_leases: u32,
    pub control_cleanup_failures: u32,
}

/// Use open_user for application traffic. Explicit private paths exist for isolated tests/tools;
/// different user-data directories are separate catalogs, never a machine-wide allocator.
pub struct PreparationAdmissionStore {
    database: Database,
    policy: PreparationAdmissionPolicy,
    recovery_root: std::path::PathBuf,
}
impl PreparationAdmissionStore {
    pub fn open_user(
        policy: PreparationAdmissionPolicy,
    ) -> Result<Self, PreparationAdmissionError> {
        user_data_directory()?;
        Self::open(user_database_path()?, policy)
    }

    pub fn open(
        path: impl AsRef<Path>,
        policy: PreparationAdmissionPolicy,
    ) -> Result<Self, PreparationAdmissionError> {
        let path = path.as_ref();
        if path.file_name() != Some(std::ffi::OsStr::new(DATABASE_FILE)) {
            return Err(PreparationAdmissionError::UserDatabase(
                UserDatabaseError::UnsafePath(path.to_owned()),
            ));
        }
        let parent = path
            .parent()
            .ok_or(PreparationAdmissionError::InvalidRequest)?;
        ensure_private_directory(parent)?;
        ensure_private_database_file(path)?;
        let mut database = Database::open(path)?;
        let transaction = database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT OR IGNORE INTO preparation_admission_policy(singleton, max_bytes, max_leases, workspace_max_bytes, workspace_max_leases) VALUES (1, ?1, ?2, ?3, ?4)",
            rusqlite::params![policy.max_bytes as i64, policy.max_leases, policy.workspace_max_bytes as i64, policy.workspace_max_leases])?;
        verify_policy(&transaction, policy)?;
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(Self {
            database,
            policy,
            recovery_root: parent.join("preparation-leases"),
        })
    }

    pub fn policy(&self) -> PreparationAdmissionPolicy {
        self.policy
    }
    pub fn usage(&self) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
        read_usage(&self.database.connection)
    }

    /// Unattributed pre-v9 rows are conservatively charged to every workspace, never discarded.
    pub fn workspace_usage(
        &self,
        workspace: &WorkspaceDirectoryIdentity,
    ) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
        read_workspace_usage(&self.database.connection, &workspace.admission_key())
    }

    /// Reserve before allocating payloads, not after. No provider, allocation or process is started.
    pub fn reserve(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        if self.policy.workspace_max_bytes != self.policy.max_bytes
            || self.policy.workspace_max_leases != self.policy.max_leases
        {
            return Err(PreparationAdmissionError::WorkspaceRequired);
        }
        self.reserve_inner(max_bytes, now_ms, None, None, false)
    }

    /// The trusted caller supplies a validated physical identity, never a model/session label.
    pub fn reserve_for_workspace(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
        workspace: &WorkspaceDirectoryIdentity,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        self.reserve_inner(
            max_bytes,
            now_ms,
            Some(workspace.admission_key()),
            None,
            false,
        )
    }

    /// Trusted preparation owner only: no payload/descriptor may leave this lease's owner before
    /// mark_exported commits. The owner must close payloads before dropping the lease/its OS lock.
    pub fn reserve_unexported_for_workspace(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
        workspace: &WorkspaceDirectoryIdentity,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        let lease = self.reserve_inner(
            max_bytes,
            now_ms,
            Some(workspace.admission_key()),
            None,
            true,
        )?;
        track_unexported(lease)
    }

    pub fn claim_queued_preparation(
        &mut self,
        ticket: &queue::PreparationQueueTicket,
        max_bytes: u64,
        now_ms: i64,
        workspace: &WorkspaceDirectoryIdentity,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        let lease = self.reserve_inner(
            max_bytes,
            now_ms,
            Some(workspace.admission_key()),
            Some(ticket),
            true,
        )?;
        track_unexported(lease)
    }
}

fn track_unexported(
    mut lease: PreparationLease<'_>,
) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
    let owner = OwnerLock::create(&lease.store.recovery_root, lease.id)?;
    let transaction = lease
        .store
        .database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Err(error) = transaction.execute("INSERT INTO preparation_admission_owners(lease_id, file_identity, exported, cleanup_confirmed) VALUES (?1, ?2, 0, 0)",
            rusqlite::params![lease.id, owner.identity().as_slice()]) {
        drop(transaction);
        // Registration never committed, so this function has not exposed any payloads. Remove only
        // its own exact empty control. An uncertain commit below deliberately retains the control.
        let _ = owner.remove();
        return Err(error.into());
    }
    transaction
        .commit()
        .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
    lease.owner = Some(owner);
    Ok(lease)
}

impl PreparationAdmissionStore {
    /// Explicit reconciliation, never a TTL sweep. Exported/legacy/raw leases are not candidates.
    pub fn recover_unexported(
        &mut self,
    ) -> Result<PreparationRecoveryReport, PreparationAdmissionError> {
        self.recover_matching(false)
    }

    /// Requires the trusted supervisor's persisted cleanup/reap proof AND absence of the owner.
    /// This cannot recover a controller lost while consumers may still be running.
    pub fn recover_completed(
        &mut self,
    ) -> Result<PreparationRecoveryReport, PreparationAdmissionError> {
        self.recover_matching(true)
    }

    fn recover_matching(
        &mut self,
        completed: bool,
    ) -> Result<PreparationRecoveryReport, PreparationAdmissionError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        let candidates = {
            let mut statement = transaction.prepare("SELECT l.lease_id, l.reserved_bytes, o.file_identity FROM preparation_admission_leases l LEFT JOIN preparation_admission_owners o USING(lease_id) WHERE ((o.exported = 0 OR (o.lease_id IS NULL AND l.preparation_intent = 1)) AND ?1 = 0) OR (o.exported = 1 AND o.cleanup_confirmed = 1 AND ?1 = 1) ORDER BY l.lease_id LIMIT 1025")?;
            let mut rows = statement.query([completed])?;
            let mut candidates = Vec::new();
            while let Some(row) = rows.next()? {
                if candidates.len() >= 1024 {
                    return Err(PreparationAdmissionError::InvalidDatabaseState);
                }
                candidates.push((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ));
            }
            candidates
        };
        let mut report = PreparationRecoveryReport::default();
        let mut locks = Vec::new();
        for (id, bytes, identity) in candidates {
            let bytes = u64::try_from(bytes)
                .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?;
            let owner = if let Some(identity) = identity {
                let identity: [u8; 16] = identity
                    .try_into()
                    .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?;
                let Some(owner) = OwnerLock::try_recover(&self.recovery_root, id, identity)? else {
                    report.busy_leases += 1;
                    continue;
                };
                Some(owner)
            } else {
                // Registration cannot commit while this IMMEDIATE transaction holds the row.
                // Losing that race makes its FK fail, before the owner is allowed to allocate.
                None
            };
            if transaction.execute(
                "DELETE FROM preparation_admission_leases WHERE lease_id = ?1",
                [id],
            )? != 1
            {
                return Err(PreparationAdmissionError::InvalidDatabaseState);
            }
            report.recovered_leases += 1;
            report.recovered_bytes = report
                .recovered_bytes
                .checked_add(bytes)
                .ok_or(PreparationAdmissionError::InvalidDatabaseState)?;
            if let Some(owner) = owner {
                locks.push(owner);
            }
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        for owner in locks {
            if owner.remove().is_err() {
                report.control_cleanup_failures += 1;
            }
        }
        Ok(report)
    }

    fn reserve_inner(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
        workspace_key: Option<[u8; 16]>,
        ticket: Option<&queue::PreparationQueueTicket>,
        preparation_intent: bool,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        if max_bytes == 0 || max_bytes > i64::MAX as u64 || now_ms < 0 {
            return Err(PreparationAdmissionError::InvalidRequest);
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        if let Some(ticket) = ticket {
            queue::validate_turn(
                &transaction,
                self.policy,
                ticket,
                workspace_key,
                max_bytes,
                now_ms,
            )?;
        } else if queue::enabled(&transaction)? {
            return Err(PreparationAdmissionError::QueueRequired);
        }
        let usage = read_usage(&transaction)?;
        let requested = usage
            .reserved_bytes
            .checked_add(max_bytes)
            .ok_or(PreparationAdmissionError::CapacityExceeded)?;
        let count = usage
            .held_leases
            .checked_add(usage.quarantined_leases)
            .ok_or(PreparationAdmissionError::InvalidDatabaseState)?;
        if requested > self.policy.max_bytes || count >= self.policy.max_leases {
            return Err(PreparationAdmissionError::CapacityExceeded);
        }
        if let Some(key) = &workspace_key {
            let usage = read_workspace_usage(&transaction, key)?;
            let requested = usage
                .reserved_bytes
                .checked_add(max_bytes)
                .ok_or(PreparationAdmissionError::WorkspaceCapacityExceeded)?;
            let count = usage
                .held_leases
                .checked_add(usage.quarantined_leases)
                .ok_or(PreparationAdmissionError::InvalidDatabaseState)?;
            if requested > self.policy.workspace_max_bytes
                || count >= self.policy.workspace_max_leases
            {
                return Err(PreparationAdmissionError::WorkspaceCapacityExceeded);
            }
        }
        transaction.execute("INSERT INTO preparation_admission_leases(reserved_bytes, created_at_ms, state, workspace_key, preparation_intent) VALUES (?1, ?2, 'held', ?3, ?4)",
            rusqlite::params![max_bytes as i64, now_ms, workspace_key.as_ref().map(|key| key.as_slice()), preparation_intent])?;
        let id = transaction.last_insert_rowid();
        if id <= 0 {
            return Err(PreparationAdmissionError::InvalidDatabaseState);
        }
        if let Some(ticket) = ticket {
            queue::consume(&transaction, ticket)?;
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(PreparationLease {
            store: self,
            id,
            reserved_bytes: max_bytes,
            released: false,
            owner: None,
        })
    }
}

fn verify_policy(
    connection: &rusqlite::Connection,
    policy: PreparationAdmissionPolicy,
) -> Result<(), PreparationAdmissionError> {
    let stored: Option<(i64, i64, i64, i64)> = connection.query_row(
        "SELECT max_bytes, max_leases, workspace_max_bytes, workspace_max_leases FROM preparation_admission_policy WHERE singleton = 1", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?;
    if stored
        != Some((
            policy.max_bytes as i64,
            i64::from(policy.max_leases),
            policy.workspace_max_bytes as i64,
            i64::from(policy.workspace_max_leases),
        ))
    {
        return Err(PreparationAdmissionError::PolicyMismatch);
    }
    Ok(())
}

fn read_workspace_usage(
    connection: &rusqlite::Connection,
    key: &[u8; 16],
) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
    let (bytes, held, quarantined): (i64, i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(reserved_bytes), 0), COALESCE(SUM(state = 'held'), 0), COALESCE(SUM(state = 'quarantined'), 0) FROM preparation_admission_leases WHERE workspace_key = ?1 OR workspace_key IS NULL", [key.as_slice()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    usage_from_counts(bytes, held, quarantined)
}

fn read_usage(
    connection: &rusqlite::Connection,
) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
    let (bytes, held, quarantined): (i64, i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(reserved_bytes), 0), COALESCE(SUM(state = 'held'), 0), COALESCE(SUM(state = 'quarantined'), 0) FROM preparation_admission_leases", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    usage_from_counts(bytes, held, quarantined)
}

fn usage_from_counts(
    bytes: i64,
    held: i64,
    quarantined: i64,
) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
    Ok(PreparationAdmissionUsage {
        reserved_bytes: u64::try_from(bytes)
            .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?,
        held_leases: u32::try_from(held)
            .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?,
        quarantined_leases: u32::try_from(quarantined)
            .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?,
    })
}

/// Opaque, one-owner reservation. Drop/crash do NOT prove payloads/consumers have stopped.
/// No Debug, Clone, serialization or recovery-by-numeric-ID API.
///
/// ```compile_fail
/// fn duplicate(lease: carapana_storage::PreparationLease<'_>) {
///     let _ = lease.clone();
/// }
/// ```
pub struct PreparationLease<'store> {
    store: &'store mut PreparationAdmissionStore,
    id: i64,
    reserved_bytes: u64,
    released: bool,
    owner: Option<OwnerLock>,
}
impl PreparationLease<'_> {
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    /// Irreversible barrier before starting a guardian or exporting payloads. An uncertain commit
    /// is an error and MUST NOT be followed by an effect. Raw leases are already non-recoverable.
    pub fn mark_exported(&self) -> Result<(), PreparationAdmissionError> {
        if let Some(owner) = &self.owner {
            // One autocommit statement is the durable barrier; this opaque lease exclusively
            // borrows the store, so a caller cannot interleave a transaction on this connection.
            if !self.store.database.connection.is_autocommit() {
                return Err(PreparationAdmissionError::InvalidDatabaseState);
            }
            if self.store.database.connection.execute("UPDATE preparation_admission_owners SET exported = 1, cleanup_confirmed = 0 WHERE lease_id = ?1 AND file_identity = ?2",
                rusqlite::params![self.id, owner.identity().as_slice()])
                .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)? != 1 {
                return Err(PreparationAdmissionError::InvalidDatabaseState);
            }
        }
        Ok(())
    }

    /// Trusted supervisor boundary only: persist AFTER independent cgroup cleanup and guardian reap.
    /// Never call this on a process error, panic, timeout of cleanup or merely because the owner died.
    pub fn mark_cleanup_confirmed(&self) -> Result<(), PreparationAdmissionError> {
        if let Some(owner) = &self.owner {
            if !self.store.database.connection.is_autocommit() {
                return Err(PreparationAdmissionError::InvalidDatabaseState);
            }
            if self.store.database.connection.execute("UPDATE preparation_admission_owners SET cleanup_confirmed = 1 WHERE lease_id = ?1 AND file_identity = ?2 AND exported = 1",
                rusqlite::params![self.id, owner.identity().as_slice()])
                .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)? != 1 {
                return Err(PreparationAdmissionError::InvalidDatabaseState);
            }
        }
        Ok(())
    }

    /// Bookkeeping boundary for the trusted owner only: close owned buffers and independently prove
    /// consumers finished FIRST. This method does not inspect processes, cgroups or RSS by itself.
    /// Failed/uncertain cleanup must drop into quarantine instead of calling this release.
    pub fn release(mut self) -> Result<(), PreparationAdmissionError> {
        let transaction = self
            .store
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute("DELETE FROM preparation_admission_leases WHERE lease_id = ?1 AND reserved_bytes = ?2 AND state = 'held'",
            rusqlite::params![self.id, self.reserved_bytes as i64])?;
        if changed != 1 {
            return Err(PreparationAdmissionError::InvalidDatabaseState);
        }
        if let Some(owner) = &self.owner {
            owner.remove()?;
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        self.released = true;
        Ok(())
    }
}
impl Drop for PreparationLease<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Even an update failure leaves the original held row charged. Never erase on Drop/TTL.
            let _ = self.store.database.connection.execute(
                "UPDATE preparation_admission_leases SET state = 'quarantined' WHERE lease_id = ?1 AND reserved_bytes = ?2",
                rusqlite::params![self.id, self.reserved_bytes as i64]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "carapana-preparation-{}-{nonce}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            }
            #[cfg(not(unix))]
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn path(&self) -> PathBuf {
            self.0.join("sessions.sqlite3")
        }
        fn open(&self, policy: PreparationAdmissionPolicy) -> PreparationAdmissionStore {
            PreparationAdmissionStore::open(self.path(), policy).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn policy(bytes: u64, slots: u32) -> PreparationAdmissionPolicy {
        PreparationAdmissionPolicy::new(bytes, slots).unwrap()
    }

    #[test]
    fn should_require_explicit_valid_limits_and_reject_invalid_requests_without_reserving() {
        for (bytes, slots) in [(0, 1), (u64::MAX, 1), (1, 0), (1, 1025)] {
            assert!(matches!(
                PreparationAdmissionPolicy::new(bytes, slots),
                Err(PreparationAdmissionError::InvalidPolicy)
            ));
        }
        let fixture = Fixture::new();
        let mut store = fixture.open(policy(100, 4));
        for (bytes, now) in [(0, 1), (u64::MAX, 1), (1, -1)] {
            assert!(matches!(
                store.reserve(bytes, now),
                Err(PreparationAdmissionError::InvalidRequest)
            ));
        }
        assert!(matches!(
            store.reserve(101, 1),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
        assert_eq!(store.policy().max_bytes(), 100);
        assert_eq!(store.policy().max_leases(), 4);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_share_byte_and_slot_capacity_across_connections_and_release_explicitly() {
        let fixture = Fixture::new();
        let mut first = fixture.open(policy(100, 2));
        let mut second = fixture.open(policy(100, 2));
        let one = first.reserve(60, 1).unwrap();
        assert_eq!(one.reserved_bytes(), 60);
        assert!(matches!(
            second.reserve(41, 2),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
        let two = second.reserve(40, 2).unwrap();
        let mut third = fixture.open(policy(100, 2));
        assert!(matches!(
            third.reserve(1, 3),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
        assert_eq!(
            third.usage().unwrap(),
            PreparationAdmissionUsage {
                reserved_bytes: 100,
                held_leases: 2,
                quarantined_leases: 0
            }
        );
        one.release().unwrap();
        two.release().unwrap();
        assert_eq!(third.usage().unwrap().reserved_bytes, 0);
        let small = first.reserve(1, 4).unwrap();
        let other = second.reserve(1, 4).unwrap();
        assert!(matches!(
            third.reserve(1, 5),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
        small.release().unwrap();
        other.release().unwrap();
    }

    #[test]
    fn should_not_silently_replace_the_shared_policy_from_another_connection() {
        let fixture = Fixture::new();
        let _store = fixture.open(policy(100, 2));
        for other in [
            policy(101, 2),
            policy(99, 2),
            policy(100, 3),
            policy(100, 1),
        ] {
            assert!(matches!(
                PreparationAdmissionStore::open(fixture.path(), other),
                Err(PreparationAdmissionError::PolicyMismatch)
            ));
        }
        assert_eq!(fixture.open(policy(100, 2)).policy(), policy(100, 2));
    }

    #[test]
    fn should_atomically_admit_only_one_competing_reservation() {
        let fixture = Fixture::new();
        let _store = fixture.open(policy(100, 4));
        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        for mut store in [fixture.open(policy(100, 4)), fixture.open(policy(100, 4))] {
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                let result = store.reserve(60, 1);
                let admitted = result.is_ok();
                barrier.wait(); // winner must not free its reservation before the other attempt.
                match result {
                    Ok(lease) => lease.release().unwrap(),
                    Err(error) => {
                        assert!(matches!(error, PreparationAdmissionError::CapacityExceeded))
                    }
                }
                admitted
            }));
        }
        let mut admitted = 0;
        for thread in threads {
            admitted += usize::from(thread.join().unwrap());
        }
        assert_eq!(admitted, 1);
        assert_eq!(
            fixture.open(policy(100, 4)).usage().unwrap().reserved_bytes,
            0
        );
    }

    #[test]
    fn should_keep_abandoned_capacity_charged_across_reopen_and_arbitrary_age() {
        let fixture = Fixture::new();
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(80, 1).unwrap());
        drop(store);
        let mut reopened = fixture.open(policy(100, 4));
        assert_eq!(
            reopened.usage().unwrap(),
            PreparationAdmissionUsage {
                reserved_bytes: 80,
                held_leases: 0,
                quarantined_leases: 1
            }
        );
        assert!(matches!(
            reopened.reserve(21, i64::MAX),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
    }

    #[test]
    fn should_keep_capacity_charged_when_release_transaction_fails() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        let lease = store.reserve_for_workspace(80, 1, &identity).unwrap();
        let observer = fixture.open(policy(100, 4));
        observer.database.connection.execute_batch("CREATE TRIGGER synthetic_release_failure BEFORE DELETE ON preparation_admission_leases BEGIN SELECT RAISE(ABORT, 'synthetic release failure'); END;").unwrap();
        assert!(matches!(
            lease.release(),
            Err(PreparationAdmissionError::Sqlite(_))
        ));
        assert_eq!(observer.usage().unwrap().reserved_bytes, 80);
        assert_eq!(observer.usage().unwrap().quarantined_leases, 1);
        assert_eq!(
            observer.workspace_usage(&identity).unwrap().reserved_bytes,
            80
        );
    }

    #[test]
    fn should_keep_held_capacity_after_process_exit_without_running_destructors() {
        const CHILD: &str = "CARAPANA_PREPARATION_EXIT_FIXTURE";
        if let Some(path) = std::env::var_os(CHILD) {
            let mut store =
                PreparationAdmissionStore::open(PathBuf::from(path), policy(100, 4)).unwrap();
            let _lease = store.reserve(80, 1).unwrap();
            std::process::exit(0);
        }
        let fixture = Fixture::new();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "preparation_admission::tests::should_keep_held_capacity_after_process_exit_without_running_destructors"])
            .env_clear().env(CHILD, fixture.path()).stdout(std::process::Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("synthetic preparation owner exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut reopened = fixture.open(policy(100, 4));
        assert_eq!(
            reopened.usage().unwrap(),
            PreparationAdmissionUsage {
                reserved_bytes: 80,
                held_leases: 1,
                quarantined_leases: 0
            }
        );
        assert!(matches!(
            reopened.reserve(21, 1000),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
    }

    #[test]
    fn should_migrate_v7_without_changing_artifacts_staging_or_sessions() {
        use crate::{ArtifactKind, ArtifactStore, SCHEMA_VERSION, SessionRegistry};
        let fixture = Fixture::new();
        let mut artifacts = ArtifactStore::open(fixture.path()).unwrap();
        let metadata = artifacts
            .write(ArtifactKind::Durable, 4, std::io::Cursor::new(b"kept"), 10)
            .unwrap();
        let mut sessions = SessionRegistry::open(fixture.path()).unwrap();
        sessions.create("kept-session", 10).unwrap();
        let before_events: i64 = rusqlite::Connection::open(fixture.path())
            .unwrap()
            .query_row("SELECT COUNT(*) FROM session_events", [], |row| row.get(0))
            .unwrap();
        let stage = artifacts.reserve_staging(64, 10).unwrap();
        let stage_path = stage.directory().to_owned();
        let connection = rusqlite::Connection::open(fixture.path()).unwrap();
        connection.execute_batch("DROP TABLE preparation_admission_leases; DROP TABLE preparation_admission_policy; PRAGMA user_version = 7;").unwrap();
        drop(connection);
        let admitted = fixture.open(policy(100, 4));
        assert_eq!(admitted.database.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(
            sessions.get("kept-session").unwrap().session_id,
            "kept-session"
        );
        let after_events: i64 = admitted
            .database
            .connection
            .query_row("SELECT COUNT(*) FROM session_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before_events, after_events);
        assert!(stage_path.exists());
        stage.directory_metadata().verify_current().unwrap();
        stage.release().unwrap();
        let mut bytes = Vec::new();
        artifacts.copy_to(metadata.id, 11, &mut bytes).unwrap();
        assert_eq!(bytes, b"kept");
        let next = artifacts
            .write(ArtifactKind::Durable, 4, std::io::Cursor::new(b"next"), 12)
            .unwrap();
        assert!(next.id > metadata.id);
    }

    #[test]
    fn should_account_preparation_separately_from_artifacts_and_refuse_alternate_catalog_names() {
        use crate::{ArtifactKind, ArtifactPolicy, ArtifactStore, ArtifactStoreError};
        let fixture = Fixture::new();
        assert!(matches!(
            PreparationAdmissionStore::open(fixture.0.join("alternate.sqlite3"), policy(100, 4)),
            Err(PreparationAdmissionError::UserDatabase(_))
        ));
        let mut store = fixture.open(policy(100, 4));
        let lease = store.reserve(80, 1).unwrap();
        let mut artifacts = ArtifactStore::open_with_policy(
            fixture.path(),
            ArtifactPolicy::new(4, 4, Duration::from_secs(1), Duration::from_secs(1)).unwrap(),
        )
        .unwrap();
        artifacts
            .write(ArtifactKind::Durable, 4, std::io::Cursor::new(b"data"), 1)
            .unwrap();
        assert!(matches!(
            artifacts.write(ArtifactKind::Durable, 1, std::io::Cursor::new(b"x"), 2),
            Err(ArtifactStoreError::QuotaExceeded { .. })
        ));
        drop(lease);
        artifacts.cleanup(1_000_000).unwrap();
        assert_eq!(store.usage().unwrap().reserved_bytes, 80);
        assert_eq!(artifacts.usage().unwrap().used_bytes, 4);
    }

    fn workspace(fixture: &Fixture, name: &str) -> WorkspaceDirectoryIdentity {
        let path = fixture.0.join(name);
        fs::create_dir(&path).unwrap();
        crate::WorkspaceMetadata::capture(path)
            .unwrap()
            .directory_identity()
    }

    #[test]
    fn should_bound_workspace_policy_and_refuse_unattributed_reservations_when_isolated() {
        for (bytes, slots) in [(0, 1), (101, 1), (10, 0), (10, 5)] {
            assert!(matches!(
                PreparationAdmissionPolicy::with_workspace_limits(100, 4, bytes, slots),
                Err(PreparationAdmissionError::InvalidPolicy)
            ));
        }
        let fixture = Fixture::new();
        let policy = PreparationAdmissionPolicy::with_workspace_limits(100, 4, 50, 2).unwrap();
        let mut store = fixture.open(policy);
        assert_eq!(policy.workspace_max_bytes(), 50);
        assert_eq!(policy.workspace_max_leases(), 2);
        assert!(matches!(
            store.reserve(1, 1),
            Err(PreparationAdmissionError::WorkspaceRequired)
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        for other in [
            PreparationAdmissionPolicy::new(100, 4).unwrap(),
            PreparationAdmissionPolicy::with_workspace_limits(100, 4, 49, 2).unwrap(),
            PreparationAdmissionPolicy::with_workspace_limits(100, 4, 50, 1).unwrap(),
        ] {
            assert!(matches!(
                PreparationAdmissionStore::open(fixture.path(), other),
                Err(PreparationAdmissionError::PolicyMismatch)
            ));
        }
    }

    #[test]
    fn should_isolate_workspace_bytes_slots_and_quarantine_across_connections() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        let policy = PreparationAdmissionPolicy::with_workspace_limits(100, 4, 60, 2).unwrap();
        let mut first = fixture.open(policy);
        let mut second = fixture.open(policy);
        let mut third = fixture.open(policy);
        let one = first.reserve_for_workspace(40, 1, &a).unwrap();
        assert!(matches!(
            second.reserve_for_workspace(21, 2, &a),
            Err(PreparationAdmissionError::WorkspaceCapacityExceeded)
        ));
        let two = second.reserve_for_workspace(20, 2, &a).unwrap();
        assert!(matches!(
            third.reserve_for_workspace(1, 3, &a),
            Err(PreparationAdmissionError::WorkspaceCapacityExceeded)
        ));
        let other = third.reserve_for_workspace(40, 3, &b).unwrap();
        let observer = fixture.open(policy);
        assert_eq!(observer.workspace_usage(&a).unwrap().reserved_bytes, 60);
        assert_eq!(observer.workspace_usage(&b).unwrap().reserved_bytes, 40);
        assert_eq!(observer.usage().unwrap().reserved_bytes, 100);
        one.release().unwrap();
        drop(two);
        other.release().unwrap();
        assert_eq!(observer.workspace_usage(&a).unwrap().quarantined_leases, 1);
        assert_eq!(observer.workspace_usage(&a).unwrap().reserved_bytes, 20);
        assert_eq!(observer.workspace_usage(&b).unwrap().reserved_bytes, 0);
        let one = first.reserve_for_workspace(1, 4, &a).unwrap();
        assert!(matches!(
            second.reserve_for_workspace(1, 4, &a),
            Err(PreparationAdmissionError::WorkspaceCapacityExceeded)
        ));
        one.release().unwrap();
        assert_eq!(observer.usage().unwrap().reserved_bytes, 20);
    }

    #[test]
    fn should_atomically_limit_competing_connections_for_the_same_physical_workspace() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let policy = PreparationAdmissionPolicy::with_workspace_limits(100, 4, 60, 2).unwrap();
        let observer = fixture.open(policy);
        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        for mut store in [fixture.open(policy), fixture.open(policy)] {
            let identity = identity.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                let result = store.reserve_for_workspace(40, 1, &identity);
                let admitted = result.is_ok();
                barrier.wait();
                match result {
                    Ok(lease) => lease.release().unwrap(),
                    Err(error) => assert!(matches!(
                        error,
                        PreparationAdmissionError::WorkspaceCapacityExceeded
                    )),
                }
                admitted
            }));
        }
        let mut admitted = 0;
        for thread in threads {
            admitted += usize::from(thread.join().unwrap());
        }
        assert_eq!(admitted, 1);
        assert_eq!(observer.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_migrate_v8_without_releasing_or_misattributing_old_leases() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        drop(fixture.open(policy(100, 4)));
        let mut artifacts = crate::ArtifactStore::open(fixture.path()).unwrap();
        let artifact = artifacts
            .write(
                crate::ArtifactKind::Durable,
                4,
                std::io::Cursor::new(b"kept"),
                1,
            )
            .unwrap();
        let mut sessions = crate::SessionRegistry::open(fixture.path()).unwrap();
        sessions.create("kept-v8-session", 1).unwrap();
        let connection = rusqlite::Connection::open(fixture.path()).unwrap();
        connection.execute_batch("DROP TABLE preparation_admission_leases; DROP TABLE preparation_admission_policy;
            CREATE TABLE preparation_admission_policy(singleton INTEGER PRIMARY KEY, max_bytes INTEGER NOT NULL, max_leases INTEGER NOT NULL) STRICT;
            CREATE TABLE preparation_admission_leases(lease_id INTEGER PRIMARY KEY AUTOINCREMENT, reserved_bytes INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, state TEXT NOT NULL) STRICT;
            INSERT INTO preparation_admission_policy VALUES(1, 100, 4);
            INSERT INTO preparation_admission_leases VALUES(1, 30, 1, 'held'), (2, 10, 2, 'quarantined');
            PRAGMA user_version = 8;").unwrap();
        drop(connection);
        let mut migrated = fixture.open(policy(100, 4));
        assert_eq!(
            migrated.database.schema_version().unwrap(),
            crate::SCHEMA_VERSION
        );
        for identity in [&a, &b] {
            assert_eq!(
                migrated.workspace_usage(identity).unwrap(),
                PreparationAdmissionUsage {
                    reserved_bytes: 40,
                    held_leases: 1,
                    quarantined_leases: 1,
                }
            );
        }
        let lease = migrated.reserve_for_workspace(60, 3, &a).unwrap();
        lease.release().unwrap();
        assert_eq!(migrated.usage().unwrap().reserved_bytes, 40);
        drop(migrated);
        let mut reopened = fixture.open(policy(100, 4));
        assert_eq!(reopened.workspace_usage(&a).unwrap().reserved_bytes, 40);
        assert!(matches!(
            reopened.reserve_for_workspace(61, i64::MAX, &b),
            Err(PreparationAdmissionError::CapacityExceeded)
        ));
        assert_eq!(
            sessions.get("kept-v8-session").unwrap().session_id,
            "kept-v8-session"
        );
        let mut bytes = Vec::new();
        artifacts.copy_to(artifact.id, 2, &mut bytes).unwrap();
        assert_eq!(bytes, b"kept");
    }

    #[test]
    fn should_recover_only_unexported_abandoned_capacity_and_preserve_live_locks() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut first = fixture.open(policy(100, 4));
        let mut observer = fixture.open(policy(100, 4));
        let lease = first
            .reserve_unexported_for_workspace(60, 1, &identity)
            .unwrap();
        let busy = observer.recover_unexported().unwrap();
        assert_eq!(busy.busy_leases, 1);
        assert_eq!(busy.recovered_leases, 0);
        assert_eq!(observer.usage().unwrap().reserved_bytes, 60);
        drop(lease);
        let report = observer.recover_unexported().unwrap();
        assert_eq!(
            report,
            PreparationRecoveryReport {
                recovered_leases: 1,
                recovered_bytes: 60,
                busy_leases: 0,
                control_cleanup_failures: 0,
            }
        );
        assert_eq!(
            observer.workspace_usage(&identity).unwrap().reserved_bytes,
            0
        );
        assert_eq!(
            observer.recover_unexported().unwrap(),
            PreparationRecoveryReport::default()
        );
        assert_eq!(fs::read_dir(&observer.recovery_root).unwrap().count(), 0);
    }

    #[test]
    fn should_never_recover_exported_or_unattributed_raw_leases() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(20, 1).unwrap());
        let lease = store
            .reserve_unexported_for_workspace(40, 2, &identity)
            .unwrap();
        lease.mark_exported().unwrap();
        lease.mark_exported().unwrap();
        drop(lease);
        drop(store);
        let mut reopened = fixture.open(policy(100, 4));
        assert_eq!(
            reopened.recover_unexported().unwrap(),
            PreparationRecoveryReport::default()
        );
        assert_eq!(reopened.usage().unwrap().reserved_bytes, 60);
        assert_eq!(reopened.usage().unwrap().quarantined_leases, 2);
    }

    #[test]
    fn should_keep_capacity_when_ownership_control_is_missing_or_replaced() {
        use std::os::unix::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        let lease = store
            .reserve_unexported_for_workspace(60, 1, &identity)
            .unwrap();
        let name = format!("{}.lock", lease.id);
        let root = lease.store.recovery_root.clone();
        drop(lease);
        let path = root.join(name);
        fs::rename(&path, root.join("original-control")).unwrap();
        assert!(matches!(
            store.recover_unexported(),
            Err(PreparationAdmissionError::RecoveryIo(_))
        ));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        assert!(matches!(
            store.recover_unexported(),
            Err(PreparationAdmissionError::RecoveryIo(_))
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 60);
    }

    #[test]
    fn should_preserve_the_lock_and_reservation_when_recovery_delete_fails() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        drop(
            store
                .reserve_unexported_for_workspace(60, 1, &identity)
                .unwrap(),
        );
        store.database.connection.execute_batch("CREATE TRIGGER synthetic_recovery_failure BEFORE DELETE ON preparation_admission_leases BEGIN SELECT RAISE(ABORT, 'synthetic recovery failure'); END;").unwrap();
        assert!(matches!(
            store.recover_unexported(),
            Err(PreparationAdmissionError::Sqlite(_))
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 60);
        assert_eq!(fs::read_dir(&store.recovery_root).unwrap().count(), 1);
        store
            .database
            .connection
            .execute_batch("DROP TRIGGER synthetic_recovery_failure")
            .unwrap();
        assert_eq!(store.recover_unexported().unwrap().recovered_bytes, 60);
    }

    #[test]
    fn should_allow_only_one_reconciler_to_recover_the_same_abandoned_lease() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        drop(
            store
                .reserve_unexported_for_workspace(60, 1, &identity)
                .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        for mut connection in [fixture.open(policy(100, 4)), fixture.open(policy(100, 4))] {
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                connection.recover_unexported().unwrap().recovered_leases
            }));
        }
        let mut recovered = 0;
        for thread in threads {
            recovered += thread.join().unwrap();
        }
        assert_eq!(recovered, 1);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_recover_a_never_exported_owner_after_exit_without_destructors() {
        const CHILD: &str = "CARAPANA_UNEXPORTED_EXIT_FIXTURE";
        if let Some(path) = std::env::var_os(CHILD) {
            let path = PathBuf::from(path);
            let identity =
                crate::WorkspaceMetadata::capture(path.parent().unwrap().join("workspace"))
                    .unwrap()
                    .directory_identity();
            let mut store = PreparationAdmissionStore::open(path, policy(100, 4)).unwrap();
            let _lease = store
                .reserve_unexported_for_workspace(60, 1, &identity)
                .unwrap();
            std::process::exit(0);
        }
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "preparation_admission::tests::should_recover_a_never_exported_owner_after_exit_without_destructors"])
            .env_clear().env(CHILD, fixture.path()).stdout(std::process::Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("synthetic unexported owner exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut reopened = fixture.open(policy(100, 4));
        assert_eq!(reopened.usage().unwrap().held_leases, 1);
        assert_eq!(reopened.recover_unexported().unwrap().recovered_bytes, 60);
        assert_eq!(
            reopened.workspace_usage(&identity).unwrap().reserved_bytes,
            0
        );
    }

    #[test]
    fn should_preserve_v9_leases_without_inventing_recovery_provenance() {
        let fixture = Fixture::new();
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(60, 1).unwrap());
        store
            .database
            .connection
            .execute_batch("DROP TABLE preparation_admission_owners; ALTER TABLE preparation_admission_leases DROP COLUMN preparation_intent; PRAGMA user_version = 9;")
            .unwrap();
        drop(store);
        let mut migrated = fixture.open(policy(100, 4));
        assert_eq!(
            migrated.database.schema_version().unwrap(),
            crate::SCHEMA_VERSION
        );
        assert_eq!(
            migrated.recover_unexported().unwrap(),
            PreparationRecoveryReport::default()
        );
        assert_eq!(migrated.usage().unwrap().reserved_bytes, 60);
    }

    #[test]
    fn should_refuse_a_failed_export_barrier_and_keep_its_lock_live() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        let mut observer = fixture.open(policy(100, 4));
        let lease = store
            .reserve_unexported_for_workspace(60, 1, &identity)
            .unwrap();
        observer.database.connection.execute_batch("CREATE TRIGGER synthetic_export_failure BEFORE UPDATE ON preparation_admission_owners BEGIN SELECT RAISE(ABORT, 'synthetic export failure'); END;").unwrap();
        assert!(matches!(
            lease.mark_exported(),
            Err(PreparationAdmissionError::CommitOutcomeUncertain)
        ));
        assert_eq!(observer.recover_unexported().unwrap().busy_leases, 1);
        drop(lease);
        assert_eq!(observer.recover_unexported().unwrap().recovered_bytes, 60);
    }

    #[test]
    fn should_reject_symlink_hardlink_and_nonprivate_ownership_controls() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for change in ["symlink", "hardlink", "permissions", "nonempty"] {
            let fixture = Fixture::new();
            let identity = workspace(&fixture, "workspace");
            let mut store = fixture.open(policy(100, 4));
            let lease = store
                .reserve_unexported_for_workspace(60, 1, &identity)
                .unwrap();
            let path = lease.store.recovery_root.join(format!("{}.lock", lease.id));
            drop(lease);
            match change {
                "symlink" => {
                    fs::rename(&path, path.with_extension("original")).unwrap();
                    symlink(path.with_extension("original"), &path).unwrap();
                }
                "hardlink" => fs::hard_link(&path, path.with_extension("other")).unwrap(),
                "permissions" => {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap()
                }
                "nonempty" => fs::write(&path, b"unexpected").unwrap(),
                _ => unreachable!(),
            }
            assert!(matches!(
                store.recover_unexported(),
                Err(PreparationAdmissionError::RecoveryIo(_))
            ));
            assert_eq!(store.usage().unwrap().reserved_bytes, 60);
        }
    }

    #[test]
    fn should_recover_completed_consumers_only_after_the_payload_owner_disappears() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        let mut observer = fixture.open(policy(100, 4));
        let lease = store
            .reserve_unexported_for_workspace(60, 1, &identity)
            .unwrap();
        assert!(matches!(
            lease.mark_cleanup_confirmed(),
            Err(PreparationAdmissionError::InvalidDatabaseState)
        ));
        lease.mark_exported().unwrap();
        assert_eq!(observer.recover_completed().unwrap().recovered_leases, 0);
        lease.mark_cleanup_confirmed().unwrap();
        assert_eq!(observer.recover_completed().unwrap().busy_leases, 1);
        assert_eq!(observer.recover_unexported().unwrap().recovered_leases, 0);
        assert_eq!(observer.usage().unwrap().reserved_bytes, 60);
        drop(lease);
        assert_eq!(observer.recover_unexported().unwrap().recovered_leases, 0);
        assert_eq!(observer.recover_completed().unwrap().recovered_bytes, 60);
        assert_eq!(observer.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_invalidate_the_completed_proof_before_a_new_export() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        let lease = store
            .reserve_unexported_for_workspace(60, 1, &identity)
            .unwrap();
        lease.mark_exported().unwrap();
        lease.mark_cleanup_confirmed().unwrap();
        lease.mark_exported().unwrap();
        drop(lease);
        assert_eq!(store.recover_unexported().unwrap().recovered_leases, 0);
        assert_eq!(store.recover_completed().unwrap().recovered_leases, 0);
        assert_eq!(store.usage().unwrap().reserved_bytes, 60);
    }

    fn queue_policy(max: u32, per: u32) -> queue::PreparationQueuePolicy {
        queue::PreparationQueuePolicy::new(max, per, Duration::from_millis(100)).unwrap()
    }

    #[test]
    fn should_bound_waiting_and_prevent_immediate_admission_bypass() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        let mut store = fixture.open(policy(100, 4));
        for (max, per, millis) in [(0, 1, 1), (1025, 1, 1), (1, 0, 1), (1, 2, 1), (1, 1, 0)] {
            assert!(
                queue::PreparationQueuePolicy::new(max, per, Duration::from_millis(millis))
                    .is_err()
            );
        }
        store.configure_queue(queue_policy(2, 1)).unwrap();
        assert!(matches!(
            store.configure_queue(queue_policy(3, 1)),
            Err(PreparationAdmissionError::PolicyMismatch)
        ));
        assert!(matches!(
            store.reserve(1, 1),
            Err(PreparationAdmissionError::QueueRequired)
        ));
        assert!(matches!(
            store.reserve_for_workspace(1, 1, &a),
            Err(PreparationAdmissionError::QueueRequired)
        ));
        let one = store.enqueue_preparation(&a, 40, 1).unwrap();
        assert_eq!(one.reserved_bytes(), 40);
        assert_eq!(one.expires_at_ms(), 101);
        assert!(matches!(
            store.enqueue_preparation(&a, 1, 1),
            Err(PreparationAdmissionError::QueueFull)
        ));
        let two = store.enqueue_preparation(&b, 40, 1).unwrap();
        assert!(matches!(
            store.enqueue_preparation(&b, 1, 1),
            Err(PreparationAdmissionError::QueueFull)
        ));
        assert_eq!(
            store.usage().unwrap().reserved_bytes,
            0,
            "waiting must not reserve payload memory"
        );
        assert!(store.cancel_preparation(&one).unwrap());
        assert!(!store.cancel_preparation(&one).unwrap());
        assert!(store.cancel_preparation(&two).unwrap());
        assert!(store.pending_preparations(1).unwrap().is_empty());
    }

    #[test]
    fn should_rotate_workspaces_preserving_fifo_and_append_newcomers_to_the_tail() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        let c = workspace(&fixture, "c");
        let d = workspace(&fixture, "d");
        let mut store = fixture.open(policy(100, 4));
        store.configure_queue(queue_policy(8, 4)).unwrap();
        let a1 = store.enqueue_preparation(&a, 10, 1).unwrap();
        let a2 = store.enqueue_preparation(&a, 10, 1).unwrap();
        let a3 = store.enqueue_preparation(&a, 10, 1).unwrap();
        let b1 = store.enqueue_preparation(&b, 10, 1).unwrap();
        let c1 = store.enqueue_preparation(&c, 10, 1).unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&a2, 10, 2, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        store
            .claim_queued_preparation(&a1, 10, 2, &a)
            .unwrap()
            .release()
            .unwrap();
        let d1 = store.enqueue_preparation(&d, 10, 2).unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&a2, 10, 2, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        for (ticket, identity) in [(&b1, &b), (&c1, &c), (&a2, &a), (&d1, &d), (&a3, &a)] {
            store
                .claim_queued_preparation(ticket, 10, 3, identity)
                .unwrap()
                .release()
                .unwrap();
        }
        assert!(store.pending_preparations(3).unwrap().is_empty());
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn should_skip_a_locally_saturated_workspace_without_losing_its_turn() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        let limits = PreparationAdmissionPolicy::with_workspace_limits(100, 4, 60, 2).unwrap();
        let mut owner = fixture.open(limits);
        let occupied = owner.reserve_for_workspace(60, 1, &a).unwrap();
        let mut store = fixture.open(limits);
        store.configure_queue(queue_policy(4, 2)).unwrap();
        let a1 = store.enqueue_preparation(&a, 40, 2).unwrap();
        let b1 = store.enqueue_preparation(&b, 40, 2).unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&a1, 40, 3, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        store
            .claim_queued_preparation(&b1, 40, 3, &b)
            .unwrap()
            .release()
            .unwrap();
        let b2 = store.enqueue_preparation(&b, 40, 4).unwrap();
        occupied.release().unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&b2, 40, 4, &b),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        store
            .claim_queued_preparation(&a1, 40, 4, &a)
            .unwrap()
            .release()
            .unwrap();
        store
            .claim_queued_preparation(&b2, 40, 4, &b)
            .unwrap()
            .release()
            .unwrap();
    }

    #[test]
    fn should_recover_only_waiting_metadata_and_expire_without_starting_or_freeing_work() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        store.configure_queue(queue_policy(4, 4)).unwrap();
        let ticket = store.enqueue_preparation(&identity, 40, 1).unwrap();
        let id = ticket.id();
        drop(store);
        let mut reopened = fixture.open(policy(100, 4));
        let recovered = reopened.pending_preparations(2).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id(), id);
        assert_eq!(reopened.usage().unwrap().reserved_bytes, 0);
        assert!(matches!(
            reopened.claim_queued_preparation(&ticket, 40, 101, &identity),
            Err(PreparationAdmissionError::QueueExpired)
        ));
        assert!(reopened.pending_preparations(101).unwrap().is_empty());
        assert!(reopened.enqueue_preparation(&identity, 0, 1).is_err());
        assert!(reopened.enqueue_preparation(&identity, 101, 1).is_err());
        assert!(
            reopened
                .enqueue_preparation(&identity, 1, i64::MAX)
                .is_err()
        );
        let next = reopened.enqueue_preparation(&identity, 40, 102).unwrap();
        let lease = reopened
            .claim_queued_preparation(&next, 40, 103, &identity)
            .unwrap();
        let mut observer = fixture.open(policy(100, 4));
        assert!(!observer.cancel_preparation(&next).unwrap());
        assert!(observer.pending_preparations(1000).unwrap().is_empty());
        assert_eq!(observer.usage().unwrap().reserved_bytes, 40);
        lease.release().unwrap();
    }

    #[test]
    fn should_reject_wrong_workspace_budget_replay_and_duplicate_competing_claims() {
        let fixture = Fixture::new();
        let a = workspace(&fixture, "a");
        let b = workspace(&fixture, "b");
        let mut store = fixture.open(policy(100, 4));
        store.configure_queue(queue_policy(4, 4)).unwrap();
        let ticket = store.enqueue_preparation(&a, 40, 1).unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&ticket, 40, 2, &b),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        assert!(matches!(
            store.claim_queued_preparation(&ticket, 41, 2, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        let mut second = fixture.open(policy(100, 4));
        let mut recovered = second.pending_preparations(2).unwrap();
        let duplicate = recovered.remove(0);
        let lease = store.claim_queued_preparation(&ticket, 40, 2, &a).unwrap();
        assert!(matches!(
            second.claim_queued_preparation(&duplicate, 40, 2, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
        assert_eq!(second.usage().unwrap().reserved_bytes, 40);
        lease.release().unwrap();
        assert!(matches!(
            second.claim_queued_preparation(&ticket, 40, 2, &a),
            Err(PreparationAdmissionError::QueueNotReady)
        ));
    }

    #[test]
    fn should_roll_back_ticket_consumption_when_reservation_insert_fails() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        store.configure_queue(queue_policy(4, 4)).unwrap();
        let ticket = store.enqueue_preparation(&identity, 40, 1).unwrap();
        store.database.connection.execute_batch("CREATE TRIGGER synthetic_claim_failure BEFORE INSERT ON preparation_admission_leases BEGIN SELECT RAISE(ABORT, 'synthetic claim failure'); END;").unwrap();
        assert!(matches!(
            store.claim_queued_preparation(&ticket, 40, 2, &identity),
            Err(PreparationAdmissionError::Sqlite(_))
        ));
        assert_eq!(store.pending_preparations(2).unwrap()[0].id(), ticket.id());
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        store
            .database
            .connection
            .execute_batch("DROP TRIGGER synthetic_claim_failure")
            .unwrap();
        store
            .claim_queued_preparation(&ticket, 40, 2, &identity)
            .unwrap()
            .release()
            .unwrap();
    }

    #[test]
    fn should_atomically_claim_a_recovered_ticket_only_once_across_threads() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        store.configure_queue(queue_policy(4, 4)).unwrap();
        store.enqueue_preparation(&identity, 40, 1).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        for mut connection in [fixture.open(policy(100, 4)), fixture.open(policy(100, 4))] {
            let ticket = connection.pending_preparations(2).unwrap().remove(0);
            let identity = identity.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                let result = connection.claim_queued_preparation(&ticket, 40, 2, &identity);
                let admitted = result.is_ok();
                barrier.wait();
                match result {
                    Ok(lease) => lease.release().unwrap(),
                    Err(error) => {
                        assert!(matches!(error, PreparationAdmissionError::QueueNotReady))
                    }
                }
                admitted
            }));
        }
        let mut admitted = 0;
        for thread in threads {
            admitted += usize::from(thread.join().unwrap());
        }
        assert_eq!(admitted, 1);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
        assert!(store.pending_preparations(2).unwrap().is_empty());
    }

    #[test]
    fn should_migrate_v10_without_enabling_waiting_or_releasing_existing_capacity() {
        let fixture = Fixture::new();
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(60, 1).unwrap());
        store.database.connection.execute_batch("DROP TABLE preparation_queue_requests; DROP TABLE preparation_queue_workspaces; DROP TABLE preparation_queue_policy; ALTER TABLE preparation_admission_leases DROP COLUMN preparation_intent; PRAGMA user_version = 10;").unwrap();
        drop(store);
        let mut migrated = fixture.open(policy(100, 4));
        assert_eq!(
            migrated.database.schema_version().unwrap(),
            crate::SCHEMA_VERSION
        );
        assert_eq!(migrated.usage().unwrap().reserved_bytes, 60);
        assert!(!queue::enabled(&migrated.database.connection).unwrap());
        migrated.reserve(40, 2).unwrap().release().unwrap();
        assert_eq!(migrated.usage().unwrap().reserved_bytes, 60);
    }

    #[test]
    fn should_reconcile_an_incomplete_preparation_before_ownership_registration_can_win() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut owner = fixture.open(policy(100, 4));
        let mut reconciler = fixture.open(policy(100, 4));
        // Pause at the exact boundary after the intent commit and before any control or payload.
        let lease = owner
            .reserve_inner(60, 1, Some(identity.admission_key()), None, true)
            .unwrap();
        assert_eq!(reconciler.recover_unexported().unwrap().recovered_bytes, 60);
        assert!(
            matches!(
                track_unexported(lease),
                Err(PreparationAdmissionError::Sqlite(_))
            ),
            "a losing registration must fail its FK rather than hand a lease to payload construction"
        );
        assert_eq!(reconciler.usage().unwrap().reserved_bytes, 0);
        assert_eq!(fs::read_dir(&reconciler.recovery_root).unwrap().count(), 0);
    }

    #[test]
    fn should_recover_a_failed_registration_without_reclassifying_raw_leases() {
        let fixture = Fixture::new();
        let identity = workspace(&fixture, "workspace");
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(60, 1).unwrap());
        store.database.connection.execute_batch("CREATE TRIGGER synthetic_registration_failure BEFORE INSERT ON preparation_admission_owners BEGIN SELECT RAISE(ABORT, 'synthetic registration failure'); END;").unwrap();
        assert!(matches!(
            store.reserve_unexported_for_workspace(30, 1, &identity),
            Err(PreparationAdmissionError::Sqlite(_))
        ));
        assert_eq!(store.usage().unwrap().reserved_bytes, 90);
        assert_eq!(store.recover_unexported().unwrap().recovered_bytes, 30);
        assert_eq!(store.usage().unwrap().reserved_bytes, 60);
    }

    #[test]
    fn should_migrate_v11_without_marking_legacy_rows_as_incomplete_preparations() {
        let fixture = Fixture::new();
        let mut store = fixture.open(policy(100, 4));
        drop(store.reserve(60, 1).unwrap());
        store.database.connection.execute_batch("ALTER TABLE preparation_admission_leases DROP COLUMN preparation_intent; PRAGMA user_version = 11;").unwrap();
        drop(store);
        let mut migrated = fixture.open(policy(100, 4));
        assert_eq!(
            migrated.database.schema_version().unwrap(),
            crate::SCHEMA_VERSION
        );
        assert_eq!(migrated.recover_unexported().unwrap().recovered_leases, 0);
        assert_eq!(migrated.usage().unwrap().reserved_bytes, 60);
    }

    #[test]
    fn should_bound_even_unregistered_control_metadata_without_deleting_unknown_files() {
        let fixture = Fixture::new();
        let root = fixture.0.join("controls");
        fs::create_dir(&root).unwrap();
        let directory = fs::File::open(&root).unwrap();
        recovery::ensure_room(&directory, 4).unwrap();
        fs::write(root.join("one"), b"").unwrap();
        fs::write(root.join("two"), b"").unwrap();
        recovery::ensure_room(&directory, 4).unwrap(); // Includes . and ..
        fs::write(root.join("three"), b"").unwrap();
        assert!(recovery::ensure_room(&directory, 4).is_err());
        assert_eq!(fs::read_dir(root).unwrap().count(), 3);
    }
}
