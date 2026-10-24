//! Shared nominal preparation reservations, not RSS measurement or proof of consumer termination.
//! The general launcher must acquire this before payload allocation and release only after cleanup.

use crate::{
    Database, MigrationError, UserDatabaseError,
    user_database::{
        DATABASE_FILE, ensure_private_database_file, ensure_private_directory, user_data_directory,
        user_database_path,
    },
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use std::{error::Error, fmt, path::Path};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preparation_admission_policy (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    max_bytes INTEGER NOT NULL CHECK (max_bytes > 0),
    max_leases INTEGER NOT NULL CHECK (max_leases > 0 AND max_leases <= 1024)
) STRICT;
CREATE TABLE IF NOT EXISTS preparation_admission_leases (
    lease_id INTEGER PRIMARY KEY AUTOINCREMENT,
    reserved_bytes INTEGER NOT NULL CHECK (reserved_bytes > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'quarantined'))
) STRICT;
"#;

/// Explicit per-user nominal capacity. No hardware-calibrated defaults are selected here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationAdmissionPolicy {
    max_bytes: u64,
    max_leases: u32,
}
impl PreparationAdmissionPolicy {
    pub fn new(max_bytes: u64, max_leases: u32) -> Result<Self, PreparationAdmissionError> {
        if max_bytes == 0 || max_bytes > i64::MAX as u64 || max_leases == 0 || max_leases > 1024 {
            return Err(PreparationAdmissionError::InvalidPolicy);
        }
        Ok(Self {
            max_bytes,
            max_leases,
        })
    }
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    pub fn max_leases(&self) -> u32 {
        self.max_leases
    }
}

#[derive(Debug)]
pub enum PreparationAdmissionError {
    InvalidPolicy,
    PolicyMismatch,
    InvalidRequest,
    CapacityExceeded,
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

/// Use open_user for application traffic. Explicit private paths exist for isolated tests/tools;
/// different user-data directories are separate catalogs, never a machine-wide allocator.
pub struct PreparationAdmissionStore {
    database: Database,
    policy: PreparationAdmissionPolicy,
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
        transaction.execute("INSERT OR IGNORE INTO preparation_admission_policy(singleton, max_bytes, max_leases) VALUES (1, ?1, ?2)",
            rusqlite::params![policy.max_bytes as i64, policy.max_leases])?;
        let stored: (i64, i64) = transaction.query_row(
            "SELECT max_bytes, max_leases FROM preparation_admission_policy WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if stored != (policy.max_bytes as i64, i64::from(policy.max_leases)) {
            return Err(PreparationAdmissionError::PolicyMismatch);
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(Self { database, policy })
    }

    pub fn policy(&self) -> PreparationAdmissionPolicy {
        self.policy
    }
    pub fn usage(&self) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
        read_usage(&self.database.connection)
    }

    /// Reserve before allocating payloads, not after. No provider, allocation or process is started.
    pub fn reserve(
        &mut self,
        max_bytes: u64,
        now_ms: i64,
    ) -> Result<PreparationLease<'_>, PreparationAdmissionError> {
        if max_bytes == 0 || max_bytes > i64::MAX as u64 || now_ms < 0 {
            return Err(PreparationAdmissionError::InvalidRequest);
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let policy: Option<(i64, i64)> = transaction.query_row("SELECT max_bytes, max_leases FROM preparation_admission_policy WHERE singleton = 1", [],
            |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        if policy
            != Some((
                self.policy.max_bytes as i64,
                i64::from(self.policy.max_leases),
            ))
        {
            return Err(PreparationAdmissionError::PolicyMismatch);
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
        transaction.execute("INSERT INTO preparation_admission_leases(reserved_bytes, created_at_ms, state) VALUES (?1, ?2, 'held')",
            rusqlite::params![max_bytes as i64, now_ms])?;
        let id = transaction.last_insert_rowid();
        if id <= 0 {
            return Err(PreparationAdmissionError::InvalidDatabaseState);
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(PreparationLease {
            store: self,
            id,
            reserved_bytes: max_bytes,
            released: false,
        })
    }
}

fn read_usage(
    connection: &rusqlite::Connection,
) -> Result<PreparationAdmissionUsage, PreparationAdmissionError> {
    let (bytes, held, quarantined): (i64, i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(reserved_bytes), 0), COALESCE(SUM(state = 'held'), 0), COALESCE(SUM(state = 'quarantined'), 0) FROM preparation_admission_leases", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
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
}
impl PreparationLease<'_> {
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
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
        let mut store = fixture.open(policy(100, 4));
        let lease = store.reserve(80, 1).unwrap();
        let observer = fixture.open(policy(100, 4));
        observer.database.connection.execute_batch("CREATE TRIGGER synthetic_release_failure BEFORE DELETE ON preparation_admission_leases BEGIN SELECT RAISE(ABORT, 'synthetic release failure'); END;").unwrap();
        assert!(matches!(
            lease.release(),
            Err(PreparationAdmissionError::Sqlite(_))
        ));
        assert_eq!(observer.usage().unwrap().reserved_bytes, 80);
        assert_eq!(observer.usage().unwrap().quarantined_leases, 1);
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
}
