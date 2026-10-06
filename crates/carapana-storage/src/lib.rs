//! SQLite schema and migration foundation for persisted local sessions.

use std::{error::Error, fmt, path::Path, time::Duration};

use rusqlite::{Connection, TransactionBehavior};

mod session_store;

pub use session_store::{
    PersistedSession, SessionStoreError, StoredSessionEvent, StoredSessionStatus,
};

/// Per-user index and lifecycle facade over the user's single SQLite database.
pub struct SessionRegistry {
    database: Database,
}

impl SessionRegistry {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MigrationError> {
        Ok(Self {
            database: Database::open(path)?,
        })
    }

    pub fn create(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        self.database.create_session(session_id, now_ms)
    }

    pub fn enqueue(
        &mut self,
        session_id: &str,
        message: carapana_protocol::QueuedMessage,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        self.database.enqueue_message(session_id, message, now_ms)
    }

    pub fn get(&self, session_id: &str) -> Result<PersistedSession, SessionStoreError> {
        self.database.load_session(session_id)
    }

    pub fn list(&mut self) -> Result<Vec<PersistedSession>, SessionStoreError> {
        self.database.list_sessions()
    }

    pub fn active(&mut self) -> Result<Vec<PersistedSession>, SessionStoreError> {
        self.database.active_sessions()
    }

    pub fn start_next(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<Option<PersistedSession>, SessionStoreError> {
        self.database.start_next_message(session_id, now_ms)
    }

    pub fn pause(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        self.database.pause_session(session_id, now_ms)
    }

    pub fn complete(
        &mut self,
        session_id: &str,
        outcome: carapana_protocol::Outcome,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        self.database
            .complete_active_message(session_id, outcome, now_ms)
    }

    pub fn recover_after_restart(
        &mut self,
        now_ms: i64,
    ) -> Result<Vec<PersistedSession>, SessionStoreError> {
        self.database.recover_active_sessions(now_ms)
    }
}

pub const SCHEMA_VERSION: i64 = 1;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(session_id)) > 0),
    state TEXT NOT NULL CHECK (state IN ('active', 'paused', 'hibernated')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms)
) STRICT;

CREATE TABLE session_events (
    session_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_type TEXT NOT NULL CHECK (length(trim(event_type)) > 0),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
    PRIMARY KEY (session_id, sequence),
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE session_snapshots (
    session_id TEXT PRIMARY KEY NOT NULL,
    event_sequence INTEGER NOT NULL CHECK (event_sequence >= 0),
    state_json TEXT NOT NULL CHECK (json_valid(state_json)),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
) STRICT;
"#;

#[derive(Debug)]
pub enum MigrationError {
    Sqlite(rusqlite::Error),
    UnsupportedSchemaVersion(i64),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite migration failed: {error}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "unsupported SQLite schema version: {version}")
            }
        }
    }
}

impl Error for MigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::UnsupportedSchemaVersion(_) => None,
        }
    }
}

impl From<rusqlite::Error> for MigrationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// A local database connection after schema migrations have been applied.
/// The caller is responsible for choosing a private, OS-owned data directory.
pub struct Database {
    connection: Connection,
}

impl Database {
    /// Create a session with an initial paused snapshot and an auditable creation event.
    pub fn create_session(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        session_store::create_session(self, session_id, now_ms)
    }

    /// Append one queued message and update the materialized snapshot atomically.
    pub fn enqueue_message(
        &mut self,
        session_id: &str,
        message: carapana_protocol::QueuedMessage,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        session_store::enqueue_message(self, session_id, message, now_ms)
    }

    /// Load from the snapshot, falling back to replaying the event log if needed.
    pub fn load_session(&self, session_id: &str) -> Result<PersistedSession, SessionStoreError> {
        session_store::load_session(self, session_id)
    }

    /// Return the durable event history in sequence order.
    pub fn session_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredSessionEvent>, SessionStoreError> {
        session_store::session_events(self, session_id)
    }

    pub fn list_sessions(&mut self) -> Result<Vec<PersistedSession>, SessionStoreError> {
        session_store::list_sessions(self, false)
    }

    pub fn active_sessions(&mut self) -> Result<Vec<PersistedSession>, SessionStoreError> {
        session_store::list_sessions(self, true)
    }

    /// Rebuild a session snapshot from its immutable event history.
    pub fn rebuild_snapshot(
        &mut self,
        session_id: &str,
    ) -> Result<PersistedSession, SessionStoreError> {
        session_store::rebuild_snapshot(self, session_id)
    }

    /// Explicitly move the oldest queued message into active state without executing it.
    pub fn start_next_message(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<Option<PersistedSession>, SessionStoreError> {
        session_store::start_next_message(self, session_id, now_ms)
    }

    /// Pause the active session while preserving its active message and remaining queue.
    pub fn pause_session(
        &mut self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        session_store::pause_session(self, session_id, now_ms)
    }

    /// Record explicit completion of the active message; this does not start the next one.
    pub fn complete_active_message(
        &mut self,
        session_id: &str,
        outcome: carapana_protocol::Outcome,
        now_ms: i64,
    ) -> Result<PersistedSession, SessionStoreError> {
        session_store::complete_active_message(self, session_id, outcome, now_ms)
    }

    /// Recover all active sessions into a paused state. No work is resumed or repeated.
    pub fn recover_active_sessions(
        &mut self,
        now_ms: i64,
    ) -> Result<Vec<PersistedSession>, SessionStoreError> {
        session_store::recover_active_sessions(self, now_ms)
    }
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MigrationError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 0 && version != SCHEMA_VERSION {
            return Err(MigrationError::UnsupportedSchemaVersion(version));
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;

        let mut database = Self { connection };
        database.migrate()?;
        Ok(database)
    }

    pub fn schema_version(&self) -> Result<i64, MigrationError> {
        self.connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(MigrationError::from)
    }

    fn migrate(&mut self) -> Result<(), MigrationError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;

        match version {
            0 => {
                transaction.execute_batch(INITIAL_SCHEMA)?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            SCHEMA_VERSION => {}
            other => return Err(MigrationError::UnsupportedSchemaVersion(other)),
        }

        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, MigrationError, SCHEMA_VERSION};
    use rusqlite::Connection;
    use std::{fs, path::PathBuf, time::SystemTime};

    struct TestDatabase(PathBuf);

    impl TestDatabase {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!("carapana-storage-{nonce}.sqlite3")))
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.0.with_extension("sqlite3-wal"));
            let _ = fs::remove_file(self.0.with_extension("sqlite3-shm"));
        }
    }

    #[test]
    fn should_migrate_and_reopen_a_real_sqlite_file_idempotently() {
        let path = TestDatabase::new();
        {
            let database = Database::open(&path.0).unwrap();
            assert_eq!(database.schema_version().unwrap(), SCHEMA_VERSION);
        }

        let database = Database::open(&path.0).unwrap();
        assert_eq!(database.schema_version().unwrap(), SCHEMA_VERSION);
        let tables: i64 = database
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('sessions', 'session_events', 'session_snapshots')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 3);

        database
            .connection
            .execute(
                "INSERT INTO sessions (session_id, state, created_at_ms, updated_at_ms) VALUES ('session-test', 'paused', 1, 1)",
                [],
            )
            .unwrap();
        assert!(database
            .connection
            .execute(
                "INSERT INTO session_events (session_id, sequence, event_type, payload_json, occurred_at_ms) VALUES ('session-test', 1, 'queued', '{not-json}', 2)",
                [],
            )
            .is_err());

        let journal_mode: String = database
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");
    }

    #[test]
    fn should_refuse_a_newer_schema_without_mutating_it() {
        let path = TestDatabase::new();
        Connection::open(&path.0)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();

        assert!(matches!(
            Database::open(&path.0),
            Err(MigrationError::UnsupportedSchemaVersion(version)) if version == SCHEMA_VERSION + 1
        ));
        let version: i64 = Connection::open(&path.0)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION + 1);
        let journal_mode: String = Connection::open(&path.0)
            .unwrap()
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "delete");
    }
}
