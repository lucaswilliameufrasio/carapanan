//! SQLite schema and migration foundation for persisted local sessions.

use std::{error::Error, fmt, path::Path, time::Duration};

use rusqlite::{Connection, TransactionBehavior};

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
