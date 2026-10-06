use std::{error::Error, fmt};

use carapana_protocol::QueuedMessage;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::Database;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredSessionStatus {
    Active,
    Paused,
    Hibernated,
}

impl StoredSessionStatus {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Hibernated => "hibernated",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedSession {
    pub session_id: String,
    pub status: StoredSessionStatus,
    pub queued_messages: Vec<QueuedMessage>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub event_sequence: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredSessionEvent {
    Created { session_id: String },
    MessageQueued { message: QueuedMessage },
}

impl StoredSessionEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Created { .. } => "created",
            Self::MessageQueued { .. } => "message_queued",
        }
    }
}

#[derive(Debug)]
pub enum SessionStoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    InvalidSessionId,
    InvalidTimestamp,
    SessionAlreadyExists,
    SessionNotFound,
    InvalidMessageId,
    DuplicateMessageId,
    InvalidEventHistory,
}

impl fmt::Display for SessionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite session operation failed: {error}"),
            Self::Json(error) => write!(formatter, "stored session JSON is invalid: {error}"),
            Self::InvalidSessionId => formatter.write_str("session id must not be blank"),
            Self::InvalidTimestamp => {
                formatter.write_str("timestamp must be non-negative and monotonic")
            }
            Self::SessionAlreadyExists => formatter.write_str("session already exists"),
            Self::SessionNotFound => formatter.write_str("session does not exist"),
            Self::InvalidMessageId => formatter.write_str("message id must not be blank"),
            Self::DuplicateMessageId => {
                formatter.write_str("message id already exists in this session")
            }
            Self::InvalidEventHistory => {
                formatter.write_str("session event history is inconsistent")
            }
        }
    }
}

impl Error for SessionStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SessionStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for SessionStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub(super) fn create_session(
    database: &mut Database,
    session_id: &str,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;

    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    if exists {
        return Err(SessionStoreError::SessionAlreadyExists);
    }

    let mut session = PersistedSession {
        session_id: session_id.to_owned(),
        status: StoredSessionStatus::Paused,
        queued_messages: Vec::new(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        event_sequence: 0,
    };
    transaction.execute(
        "INSERT INTO sessions (session_id, state, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?3)",
        params![session_id, session.status.as_db_value(), now_ms],
    )?;
    append_event(
        &transaction,
        &mut session,
        StoredSessionEvent::Created {
            session_id: session_id.to_owned(),
        },
        now_ms,
    )?;
    write_snapshot(&transaction, &session)?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn enqueue_message(
    database: &mut Database,
    session_id: &str,
    message: QueuedMessage,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;
    if message.id.trim().is_empty() {
        return Err(SessionStoreError::InvalidMessageId);
    }

    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut session = load_session_tx(&transaction, session_id)?;
    if now_ms < session.updated_at_ms {
        return Err(SessionStoreError::InvalidTimestamp);
    }
    if session
        .queued_messages
        .iter()
        .any(|queued| queued.id == message.id)
    {
        return Err(SessionStoreError::DuplicateMessageId);
    }
    session.queued_messages.push(message.clone());
    append_event(
        &transaction,
        &mut session,
        StoredSessionEvent::MessageQueued { message },
        now_ms,
    )?;
    transaction.execute(
        "UPDATE sessions SET updated_at_ms = ?2 WHERE session_id = ?1",
        params![session_id, now_ms],
    )?;
    write_snapshot(&transaction, &session)?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn load_session(
    database: &Database,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    load_session_connection(&database.connection, session_id)
}

pub(super) fn session_events(
    database: &Database,
    session_id: &str,
) -> Result<Vec<StoredSessionEvent>, SessionStoreError> {
    validate_session_id(session_id)?;
    let exists: bool = database.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(SessionStoreError::SessionNotFound);
    }
    read_events(&database.connection, session_id)
}

pub(super) fn rebuild_snapshot(
    database: &mut Database,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session = replay_events(&transaction, session_id)?;
    let updated = transaction.execute(
        "UPDATE sessions SET state = ?2, created_at_ms = ?3, updated_at_ms = ?4 WHERE session_id = ?1",
        params![
            session_id,
            session.status.as_db_value(),
            session.created_at_ms,
            session.updated_at_ms
        ],
    )?;
    if updated != 1 {
        return Err(SessionStoreError::SessionNotFound);
    }
    write_snapshot(&transaction, &session)?;
    transaction.commit()?;
    Ok(session)
}

fn load_session_connection(
    connection: &rusqlite::Connection,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    let session_row: Option<(String, i64, i64)> = connection
        .query_row(
            "SELECT state, created_at_ms, updated_at_ms FROM sessions WHERE session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((state, created_at_ms, updated_at_ms)) = session_row else {
        return Err(SessionStoreError::SessionNotFound);
    };
    let expected_status = match state.as_str() {
        "active" => StoredSessionStatus::Active,
        "paused" => StoredSessionStatus::Paused,
        "hibernated" => StoredSessionStatus::Hibernated,
        _ => return Err(SessionStoreError::InvalidEventHistory),
    };
    let latest_sequence: i64 = connection.query_row(
        "SELECT COALESCE(MAX(sequence), 0) FROM session_events WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    let snapshot: Option<String> = connection
        .query_row(
            "SELECT state_json FROM session_snapshots WHERE session_id = ?1 AND event_sequence = ?2",
            params![session_id, latest_sequence],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(snapshot) = snapshot
        && let Ok(session) = serde_json::from_str::<PersistedSession>(&snapshot)
        && session.session_id == session_id
        && session.created_at_ms == created_at_ms
        && session.updated_at_ms == updated_at_ms
        && session.status == expected_status
        && session.event_sequence == latest_sequence
    {
        return Ok(session);
    }

    let replayed = replay_events(connection, session_id)?;
    if replayed.created_at_ms != created_at_ms
        || replayed.updated_at_ms != updated_at_ms
        || replayed.status != expected_status
        || replayed.event_sequence != latest_sequence
    {
        return Err(SessionStoreError::InvalidEventHistory);
    }
    Ok(replayed)
}

fn load_session_tx(
    transaction: &Transaction<'_>,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    load_session_connection(transaction, session_id)
}

fn replay_events(
    connection: &rusqlite::Connection,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_type, payload_json, occurred_at_ms FROM session_events WHERE session_id = ?1 ORDER BY sequence",
    )?;
    let mut rows = statement.query([session_id])?;
    let mut session: Option<PersistedSession> = None;
    let mut expected_sequence = 1_i64;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let event_type: String = row.get(1)?;
        let payload: String = row.get(2)?;
        let occurred_at_ms: i64 = row.get(3)?;
        if sequence != expected_sequence {
            return Err(SessionStoreError::InvalidEventHistory);
        }
        let event: StoredSessionEvent = serde_json::from_str(&payload)?;
        if event.event_type() != event_type {
            return Err(SessionStoreError::InvalidEventHistory);
        }
        apply_event(&mut session, session_id, event, occurred_at_ms, sequence)?;
        expected_sequence += 1;
    }
    session.ok_or(SessionStoreError::SessionNotFound)
}

fn read_events(
    connection: &rusqlite::Connection,
    session_id: &str,
) -> Result<Vec<StoredSessionEvent>, SessionStoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_type, payload_json FROM session_events WHERE session_id = ?1 ORDER BY sequence",
    )?;
    let mut rows = statement.query([session_id])?;
    let mut events = Vec::new();
    let mut expected_sequence = 1_i64;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let event_type: String = row.get(1)?;
        let payload: String = row.get(2)?;
        if sequence != expected_sequence {
            return Err(SessionStoreError::InvalidEventHistory);
        }
        let event: StoredSessionEvent = serde_json::from_str(&payload)?;
        if event.event_type() != event_type {
            return Err(SessionStoreError::InvalidEventHistory);
        }
        events.push(event);
        expected_sequence += 1;
    }
    Ok(events)
}

fn append_event(
    transaction: &Transaction<'_>,
    session: &mut PersistedSession,
    event: StoredSessionEvent,
    occurred_at_ms: i64,
) -> Result<(), SessionStoreError> {
    let next_sequence = session.event_sequence + 1;
    let payload_json = serde_json::to_string(&event)?;
    transaction.execute(
        "INSERT INTO session_events (session_id, sequence, event_type, payload_json, occurred_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            session.session_id,
            next_sequence,
            event.event_type(),
            payload_json,
            occurred_at_ms
        ],
    )?;
    session.event_sequence = next_sequence;
    session.updated_at_ms = occurred_at_ms;
    Ok(())
}

fn apply_event(
    session: &mut Option<PersistedSession>,
    session_id: &str,
    event: StoredSessionEvent,
    occurred_at_ms: i64,
    sequence: i64,
) -> Result<(), SessionStoreError> {
    match (session.as_mut(), event) {
        (
            None,
            StoredSessionEvent::Created {
                session_id: created_id,
            },
        ) if created_id == session_id && sequence == 1 => {
            *session = Some(PersistedSession {
                session_id: created_id,
                status: StoredSessionStatus::Paused,
                queued_messages: Vec::new(),
                created_at_ms: occurred_at_ms,
                updated_at_ms: occurred_at_ms,
                event_sequence: sequence,
            });
        }
        (Some(current), StoredSessionEvent::MessageQueued { message })
            if sequence == current.event_sequence + 1
                && occurred_at_ms >= current.updated_at_ms
                && !message.id.trim().is_empty()
                && !current
                    .queued_messages
                    .iter()
                    .any(|queued| queued.id == message.id) =>
        {
            current.queued_messages.push(message);
            current.updated_at_ms = occurred_at_ms;
            current.event_sequence = sequence;
        }
        _ => return Err(SessionStoreError::InvalidEventHistory),
    }
    Ok(())
}

fn write_snapshot(
    transaction: &Transaction<'_>,
    session: &PersistedSession,
) -> Result<(), SessionStoreError> {
    let state_json = serde_json::to_string(session)?;
    transaction.execute(
        "INSERT INTO session_snapshots (session_id, event_sequence, state_json, updated_at_ms) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(session_id) DO UPDATE SET event_sequence = excluded.event_sequence, state_json = excluded.state_json, updated_at_ms = excluded.updated_at_ms",
        params![
            session.session_id,
            session.event_sequence,
            state_json,
            session.updated_at_ms
        ],
    )?;
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<(), SessionStoreError> {
    if session_id.trim().is_empty() {
        Err(SessionStoreError::InvalidSessionId)
    } else {
        Ok(())
    }
}

fn validate_timestamp(timestamp_ms: i64) -> Result<(), SessionStoreError> {
    if timestamp_ms < 0 {
        Err(SessionStoreError::InvalidTimestamp)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionStoreError, StoredSessionEvent, StoredSessionStatus};
    use crate::Database;
    use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
    use rusqlite::params;
    use std::{fs, path::PathBuf, time::SystemTime};

    struct TestDatabase(PathBuf);

    impl TestDatabase {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!("carapana-session-{nonce}.sqlite3")))
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.0.with_extension("sqlite3-wal"));
            let _ = fs::remove_file(self.0.with_extension("sqlite3-shm"));
        }
    }

    fn message(id: &str) -> QueuedMessage {
        QueuedMessage {
            id: id.into(),
            text: format!("message {id}"),
            origin: "test".into(),
            selection: Selection {
                profile: "careful".into(),
                work: WorkMode::Plan,
                autonomy: Autonomy::Ask,
                provider: "mock".into(),
                model: "mock-model".into(),
                variant: "default".into(),
            },
        }
    }

    #[test]
    fn should_persist_queue_selection_and_order_through_a_real_database_reopen() {
        let path = TestDatabase::new();
        {
            let mut database = Database::open(&path.0).unwrap();
            database.create_session("session-1", 10).unwrap();
            database
                .enqueue_message("session-1", message("m1"), 11)
                .unwrap();
            let session = database
                .enqueue_message("session-1", message("m2"), 12)
                .unwrap();
            assert_eq!(session.event_sequence, 3);
            assert_eq!(session.queued_messages.len(), 2);
            assert_eq!(session.status, StoredSessionStatus::Paused);
        }

        let database = Database::open(&path.0).unwrap();
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.queued_messages, vec![message("m1"), message("m2")]);
        assert_eq!(session.queued_messages[0].selection.profile, "careful");
        assert_eq!(session.event_sequence, 3);
        assert_eq!(
            database.session_events("session-1").unwrap(),
            vec![
                StoredSessionEvent::Created {
                    session_id: "session-1".into()
                },
                StoredSessionEvent::MessageQueued {
                    message: message("m1")
                },
                StoredSessionEvent::MessageQueued {
                    message: message("m2")
                }
            ]
        );
    }

    #[test]
    fn should_rollback_duplicate_or_non_monotonic_queue_mutations() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        database
            .enqueue_message("session-1", message("m1"), 11)
            .unwrap();

        assert!(matches!(
            database.enqueue_message("session-1", message("m1"), 12),
            Err(SessionStoreError::DuplicateMessageId)
        ));
        assert!(matches!(
            database.enqueue_message("session-1", message("m2"), 9),
            Err(SessionStoreError::InvalidTimestamp)
        ));
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.event_sequence, 2);
        assert_eq!(session.queued_messages, vec![message("m1")]);
        assert_eq!(database.session_events("session-1").unwrap().len(), 2);
    }

    #[test]
    fn should_rebuild_a_missing_or_corrupt_snapshot_from_contiguous_events() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        database
            .enqueue_message("session-1", message("m1"), 11)
            .unwrap();
        database
            .connection
            .execute(
                "UPDATE session_snapshots SET state_json = ?1 WHERE session_id = ?2",
                params!["{\"corrupt\":true}", "session-1"],
            )
            .unwrap();
        database
            .connection
            .execute(
                "UPDATE sessions SET updated_at_ms = 99 WHERE session_id = 'session-1'",
                [],
            )
            .unwrap();

        assert!(matches!(
            database.load_session("session-1"),
            Err(SessionStoreError::InvalidEventHistory)
        ));
        let rebuilt = database.rebuild_snapshot("session-1").unwrap();
        assert_eq!(rebuilt.queued_messages, vec![message("m1")]);
        assert_eq!(rebuilt.updated_at_ms, 11);
        let snapshot_json: String = database
            .connection
            .query_row(
                "SELECT state_json FROM session_snapshots WHERE session_id = 'session-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(serde_json::from_str::<super::super::PersistedSession>(&snapshot_json).is_ok());
    }
}
