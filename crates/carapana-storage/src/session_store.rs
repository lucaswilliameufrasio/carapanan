use std::{collections::HashSet, error::Error, fmt};

use carapana_protocol::{Outcome, QueuedMessage};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{
    Database, WorkspaceFileMetadata, WorkspaceFileMetadataError, WorkspaceMetadata,
    WorkspaceMetadataError,
};

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
    #[serde(default)]
    pub active_message: Option<QueuedMessage>,
    #[serde(default)]
    pub recovery_needs_revalidation: bool,
    #[serde(default)]
    pub active_work_uncertain: bool,
    #[serde(default)]
    pub workspace: Option<WorkspaceMetadata>,
    #[serde(default)]
    pub workspace_files: Vec<WorkspaceFileMetadata>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub event_sequence: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredSessionEvent {
    Created {
        session_id: String,
        #[serde(default)]
        workspace: Option<WorkspaceMetadata>,
    },
    WorkspaceFileObserved {
        observation: WorkspaceFileMetadata,
    },
    MessageQueued {
        message: QueuedMessage,
    },
    MessageStarted {
        message_id: String,
    },
    Paused,
    Completed {
        message_id: String,
        outcome: Outcome,
    },
    RecoveredPaused {
        active_message_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSessionEventRecord {
    pub sequence: i64,
    pub occurred_at_ms: i64,
    pub event: StoredSessionEvent,
}

impl StoredSessionEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Created { .. } => "created",
            Self::WorkspaceFileObserved { .. } => "workspace_file_observed",
            Self::MessageQueued { .. } => "message_queued",
            Self::MessageStarted { .. } => "message_started",
            Self::Paused => "paused",
            Self::Completed { .. } => "completed",
            Self::RecoveredPaused { .. } => "recovered_paused",
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
    InvalidEventCursor,
    InvalidTransition,
    RevalidationRequired,
    NoActiveMessage,
    WorkspaceMetadata(WorkspaceMetadataError),
    WorkspaceFileMetadata(WorkspaceFileMetadataError),
    WorkspaceNotConfigured,
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
            Self::InvalidEventCursor => {
                formatter.write_str("session event cursor is outside the event history")
            }
            Self::InvalidTransition => {
                formatter.write_str("session transition is not valid in the current state")
            }
            Self::RevalidationRequired => {
                formatter.write_str("session state must be revalidated before advancing")
            }
            Self::NoActiveMessage => formatter.write_str("session has no active message"),
            Self::WorkspaceMetadata(error) => {
                write!(formatter, "invalid session workspace: {error}")
            }
            Self::WorkspaceFileMetadata(error) => {
                write!(formatter, "invalid session workspace file: {error}")
            }
            Self::WorkspaceNotConfigured => {
                formatter.write_str("session has no configured workspace")
            }
        }
    }
}

impl Error for SessionStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::WorkspaceMetadata(error) => Some(error),
            Self::WorkspaceFileMetadata(error) => Some(error),
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
    create_session_with_optional_workspace(database, session_id, None, now_ms)
}

pub(super) fn create_session_with_workspace(
    database: &mut Database,
    session_id: &str,
    workspace: WorkspaceMetadata,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    workspace
        .verify_current()
        .map_err(SessionStoreError::WorkspaceMetadata)?;
    create_session_with_optional_workspace(database, session_id, Some(workspace), now_ms)
}

fn create_session_with_optional_workspace(
    database: &mut Database,
    session_id: &str,
    workspace: Option<WorkspaceMetadata>,
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
        active_message: None,
        recovery_needs_revalidation: false,
        active_work_uncertain: false,
        workspace: workspace.clone(),
        workspace_files: Vec::new(),
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
            workspace,
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
    let duplicate_id: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_events WHERE session_id = ?1 AND event_type = 'message_queued' AND json_extract(payload_json, '$.message.id') = ?2)",
        rusqlite::params![session_id, message.id],
        |row| row.get(0),
    )?;
    if duplicate_id {
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

pub(super) fn observe_workspace_file(
    database: &mut Database,
    session_id: &str,
    relative_path: impl AsRef<std::path::Path>,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    record_workspace_file(database, session_id, relative_path, now_ms, false)
}

pub(super) fn observe_workspace_file_with_hash(
    database: &mut Database,
    session_id: &str,
    relative_path: impl AsRef<std::path::Path>,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    record_workspace_file(database, session_id, relative_path, now_ms, true)
}

fn record_workspace_file(
    database: &mut Database,
    session_id: &str,
    relative_path: impl AsRef<std::path::Path>,
    now_ms: i64,
    with_hash: bool,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut session = load_session_tx(&transaction, session_id)?;
    validate_transition_time(&session, now_ms)?;
    let workspace = session
        .workspace
        .as_ref()
        .ok_or(SessionStoreError::WorkspaceNotConfigured)?;
    let observation = if with_hash {
        workspace.observe_file_with_hash(relative_path)
    } else {
        workspace.observe_file(relative_path)
    }
    .map_err(SessionStoreError::WorkspaceFileMetadata)?;
    apply_and_persist(
        &transaction,
        &mut session,
        StoredSessionEvent::WorkspaceFileObserved { observation },
        now_ms,
    )?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn validate_workspace_files(
    database: &Database,
    session_id: &str,
) -> Result<(), SessionStoreError> {
    let session = database.load_session(session_id)?;
    let workspace = session
        .workspace
        .as_ref()
        .ok_or(SessionStoreError::WorkspaceNotConfigured)?;
    workspace
        .verify_current()
        .map_err(SessionStoreError::WorkspaceMetadata)?;
    for observation in &session.workspace_files {
        observation
            .verify_current(workspace)
            .map_err(SessionStoreError::WorkspaceFileMetadata)?;
    }
    Ok(())
}

pub(super) fn load_session(
    database: &Database,
    session_id: &str,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    load_session_connection(&database.connection, session_id)
}

pub(super) fn list_sessions(
    database: &mut Database,
    active_only: bool,
) -> Result<Vec<PersistedSession>, SessionStoreError> {
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Deferred)?;
    let session_ids = {
        let mut statement = transaction.prepare(
            "SELECT session_id FROM sessions WHERE (?1 = 0 OR state = 'active') ORDER BY updated_at_ms DESC, session_id ASC",
        )?;
        let rows = statement.query_map([active_only], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let sessions = session_ids
        .iter()
        .map(|session_id| load_session_connection(&transaction, session_id))
        .collect::<Result<Vec<_>, _>>()?;
    transaction.commit()?;
    Ok(sessions)
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

pub(super) fn session_events_after(
    database: &mut Database,
    session_id: &str,
    after_sequence: i64,
    limit: u16,
) -> Result<(Vec<StoredSessionEventRecord>, i64, bool), SessionStoreError> {
    validate_session_id(session_id)?;
    if after_sequence < 0 || limit == 0 {
        return Err(SessionStoreError::InvalidEventCursor);
    }

    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Deferred)?;
    let current_sequence = transaction
        .query_row(
            "SELECT event_sequence FROM session_snapshots WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(SessionStoreError::SessionNotFound)?;
    if after_sequence > current_sequence {
        return Err(SessionStoreError::InvalidEventCursor);
    }

    let (records, has_more) = {
        let fetch_limit = i64::from(limit) + 1;
        let mut statement = transaction.prepare(
            "SELECT sequence, occurred_at_ms, event_type, payload_json FROM session_events WHERE session_id = ?1 AND sequence > ?2 ORDER BY sequence LIMIT ?3",
        )?;
        let mut rows = statement.query(params![session_id, after_sequence, fetch_limit])?;
        let mut records = Vec::new();
        let mut expected_sequence = after_sequence
            .checked_add(1)
            .ok_or(SessionStoreError::InvalidEventCursor)?;
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let occurred_at_ms: i64 = row.get(1)?;
            let event_type: String = row.get(2)?;
            let payload: String = row.get(3)?;
            if sequence != expected_sequence {
                return Err(SessionStoreError::InvalidEventHistory);
            }
            let event: StoredSessionEvent = serde_json::from_str(&payload)?;
            if event.event_type() != event_type {
                return Err(SessionStoreError::InvalidEventHistory);
            }
            records.push(StoredSessionEventRecord {
                sequence,
                occurred_at_ms,
                event,
            });
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or(SessionStoreError::InvalidEventHistory)?;
        }
        let has_more = records.len() > usize::from(limit);
        records.truncate(usize::from(limit));
        (records, has_more)
    };
    let next_sequence = records
        .last()
        .map_or(after_sequence, |record| record.sequence);
    transaction.commit()?;
    Ok((records, next_sequence, has_more))
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

pub(super) fn start_next_message(
    database: &mut Database,
    session_id: &str,
    now_ms: i64,
) -> Result<Option<PersistedSession>, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut session = load_session_tx(&transaction, session_id)?;
    validate_transition_time(&session, now_ms)?;
    if session.recovery_needs_revalidation {
        return Err(SessionStoreError::RevalidationRequired);
    }
    if session.status != StoredSessionStatus::Paused || session.active_message.is_some() {
        return Err(SessionStoreError::InvalidTransition);
    }
    let Some(message_id) = session
        .queued_messages
        .first()
        .map(|message| message.id.clone())
    else {
        return Ok(None);
    };
    apply_and_persist(
        &transaction,
        &mut session,
        StoredSessionEvent::MessageStarted { message_id },
        now_ms,
    )?;
    transaction.commit()?;
    Ok(Some(session))
}

pub(super) fn pause_session(
    database: &mut Database,
    session_id: &str,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut session = load_session_tx(&transaction, session_id)?;
    validate_transition_time(&session, now_ms)?;
    if session.status != StoredSessionStatus::Active || session.active_message.is_none() {
        return Err(SessionStoreError::InvalidTransition);
    }
    apply_and_persist(
        &transaction,
        &mut session,
        StoredSessionEvent::Paused,
        now_ms,
    )?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn complete_active_message(
    database: &mut Database,
    session_id: &str,
    outcome: Outcome,
    now_ms: i64,
) -> Result<PersistedSession, SessionStoreError> {
    validate_session_id(session_id)?;
    validate_timestamp(now_ms)?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut session = load_session_tx(&transaction, session_id)?;
    validate_transition_time(&session, now_ms)?;
    if session.status != StoredSessionStatus::Active {
        return Err(SessionStoreError::InvalidTransition);
    }
    let message_id = session
        .active_message
        .as_ref()
        .ok_or(SessionStoreError::NoActiveMessage)?
        .id
        .clone();
    apply_and_persist(
        &transaction,
        &mut session,
        StoredSessionEvent::Completed {
            message_id,
            outcome,
        },
        now_ms,
    )?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn recover_active_sessions(
    database: &mut Database,
    now_ms: i64,
) -> Result<Vec<PersistedSession>, SessionStoreError> {
    validate_timestamp(now_ms)?;
    let active_ids = {
        let mut statement = database.connection.prepare(
            "SELECT session_id FROM sessions WHERE state = 'active' ORDER BY session_id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut recovered = Vec::with_capacity(active_ids.len());
    for session_id in active_ids {
        let transaction = database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut session = load_session_tx(&transaction, &session_id)?;
        if session.status != StoredSessionStatus::Active {
            transaction.commit()?;
            continue;
        }
        validate_transition_time(&session, now_ms)?;
        let active_message_id = session
            .active_message
            .as_ref()
            .map(|message| message.id.clone());
        apply_and_persist(
            &transaction,
            &mut session,
            StoredSessionEvent::RecoveredPaused { active_message_id },
            now_ms,
        )?;
        transaction.commit()?;
        recovered.push(session);
    }
    Ok(recovered)
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
    let mut seen_message_ids = HashSet::new();
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
        if let StoredSessionEvent::MessageQueued { message } = &event
            && !seen_message_ids.insert(message.id.clone())
        {
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
                workspace,
            },
        ) if created_id == session_id && sequence == 1 => {
            *session = Some(PersistedSession {
                session_id: created_id,
                status: StoredSessionStatus::Paused,
                queued_messages: Vec::new(),
                active_message: None,
                recovery_needs_revalidation: false,
                active_work_uncertain: false,
                workspace,
                workspace_files: Vec::new(),
                created_at_ms: occurred_at_ms,
                updated_at_ms: occurred_at_ms,
                event_sequence: sequence,
            });
        }
        (Some(current), event) => apply_to_session(current, &event, occurred_at_ms, sequence)?,
        _ => return Err(SessionStoreError::InvalidEventHistory),
    }
    Ok(())
}

fn apply_and_persist(
    transaction: &Transaction<'_>,
    session: &mut PersistedSession,
    event: StoredSessionEvent,
    occurred_at_ms: i64,
) -> Result<(), SessionStoreError> {
    let next_sequence = session.event_sequence + 1;
    let mut updated_session = session.clone();
    apply_to_session(&mut updated_session, &event, occurred_at_ms, next_sequence)?;
    let payload_json = serde_json::to_string(&event)?;
    transaction.execute(
        "INSERT INTO session_events (session_id, sequence, event_type, payload_json, occurred_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            updated_session.session_id,
            next_sequence,
            event.event_type(),
            payload_json,
            occurred_at_ms
        ],
    )?;
    transaction.execute(
        "UPDATE sessions SET state = ?2, updated_at_ms = ?3 WHERE session_id = ?1",
        params![
            updated_session.session_id,
            updated_session.status.as_db_value(),
            occurred_at_ms
        ],
    )?;
    write_snapshot(transaction, &updated_session)?;
    *session = updated_session;
    Ok(())
}

fn apply_to_session(
    session: &mut PersistedSession,
    event: &StoredSessionEvent,
    occurred_at_ms: i64,
    sequence: i64,
) -> Result<(), SessionStoreError> {
    if occurred_at_ms < session.updated_at_ms || sequence != session.event_sequence + 1 {
        return Err(SessionStoreError::InvalidEventHistory);
    }
    match event {
        StoredSessionEvent::WorkspaceFileObserved { observation } => {
            if session.workspace.is_none() {
                return Err(SessionStoreError::WorkspaceNotConfigured);
            }
            observation
                .validate_path()
                .map_err(SessionStoreError::WorkspaceFileMetadata)?;
            let path = observation.relative_path();
            session
                .workspace_files
                .retain(|current| current.relative_path() != path);
            session.workspace_files.push(observation.clone());
        }
        StoredSessionEvent::MessageQueued { message }
            if !message.id.trim().is_empty()
                && session
                    .active_message
                    .as_ref()
                    .is_none_or(|active| active.id != message.id)
                && !session
                    .queued_messages
                    .iter()
                    .any(|queued| queued.id == message.id) =>
        {
            session.queued_messages.push(message.clone());
        }
        StoredSessionEvent::MessageStarted { message_id }
            if session.status == StoredSessionStatus::Paused
                && session.active_message.is_none()
                && session
                    .queued_messages
                    .first()
                    .is_some_and(|message| &message.id == message_id) =>
        {
            session.active_message = Some(session.queued_messages.remove(0));
            session.status = StoredSessionStatus::Active;
            session.recovery_needs_revalidation = false;
            session.active_work_uncertain = false;
        }
        StoredSessionEvent::Paused
            if session.status == StoredSessionStatus::Active
                && session.active_message.is_some() =>
        {
            session.status = StoredSessionStatus::Paused;
        }
        StoredSessionEvent::Completed { message_id, .. }
            if session.status == StoredSessionStatus::Active
                && session
                    .active_message
                    .as_ref()
                    .is_some_and(|message| &message.id == message_id) =>
        {
            session.active_message = None;
            session.status = StoredSessionStatus::Paused;
            session.recovery_needs_revalidation = false;
            session.active_work_uncertain = false;
        }
        StoredSessionEvent::RecoveredPaused { active_message_id }
            if session.status == StoredSessionStatus::Active
                && session.active_message.as_ref().map(|message| &message.id)
                    == active_message_id.as_ref() =>
        {
            session.status = StoredSessionStatus::Paused;
            session.recovery_needs_revalidation = true;
            session.active_work_uncertain = session.active_message.is_some();
        }
        _ => return Err(SessionStoreError::InvalidEventHistory),
    }
    session.updated_at_ms = occurred_at_ms;
    session.event_sequence = sequence;
    Ok(())
}

fn validate_transition_time(
    session: &PersistedSession,
    now_ms: i64,
) -> Result<(), SessionStoreError> {
    if now_ms < session.updated_at_ms {
        Err(SessionStoreError::InvalidTimestamp)
    } else {
        Ok(())
    }
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
    use crate::{
        Database, SessionRegistry, WorkspaceFileMetadataError, WorkspaceMetadata,
        WorkspaceMetadataError,
    };
    use carapana_protocol::{Autonomy, Outcome, QueuedMessage, Selection, WorkMode};
    use rusqlite::params;
    use std::{
        fs,
        fs::OpenOptions,
        path::PathBuf,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
        thread,
        time::SystemTime,
    };

    struct TestDatabase(PathBuf);

    struct TestWorkspace(PathBuf);

    impl TestDatabase {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self::with_timestamp(nonce)
        }

        fn with_timestamp(nonce: u128) -> Self {
            Self(test_database_path(nonce))
        }
    }

    impl TestWorkspace {
        fn new() -> Self {
            static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-session-workspace-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    fn test_database_path(nonce: u128) -> PathBuf {
        static NEXT_TEST_DATABASE_ID: AtomicU64 = AtomicU64::new(0);
        loop {
            let id = NEXT_TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-session-{}-{nonce}-{id}.sqlite3",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    return path;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not allocate SQLite test fixture: {error}"),
            }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.0.with_extension("sqlite3-wal"));
            let _ = fs::remove_file(self.0.with_extension("sqlite3-shm"));
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
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
    fn should_allocate_distinct_database_files_when_clock_timestamps_collide() {
        let first = TestDatabase::with_timestamp(42);
        let second = TestDatabase::with_timestamp(42);
        assert_ne!(first.0, second.0);
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
                    session_id: "session-1".into(),
                    workspace: None,
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

    #[cfg(unix)]
    #[test]
    fn should_persist_workspace_identity_in_event_history_and_rebuilt_snapshot() {
        let path = TestDatabase::new();
        let workspace = TestWorkspace::new();
        let metadata = WorkspaceMetadata::capture(&workspace.0).unwrap();
        {
            let mut database = Database::open(&path.0).unwrap();
            let created = database
                .create_session_with_workspace("session-1", metadata.clone(), 10)
                .unwrap();
            assert_eq!(created.workspace, Some(metadata.clone()));
            assert!(matches!(
                database.session_events("session-1").unwrap().first(),
                Some(StoredSessionEvent::Created {
                    workspace: Some(saved),
                    ..
                }) if saved == &metadata
            ));
        }

        let mut database = Database::open(&path.0).unwrap();
        let loaded = database.load_session("session-1").unwrap();
        assert_eq!(loaded.workspace, Some(metadata.clone()));
        let rebuilt = database.rebuild_snapshot("session-1").unwrap();
        assert_eq!(rebuilt.workspace, Some(metadata));
    }

    #[cfg(unix)]
    #[test]
    fn should_persist_and_revalidate_explicit_workspace_file_observations_after_reopen() {
        let path = TestDatabase::new();
        let workspace = TestWorkspace::new();
        let file = workspace.0.join("src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"stat-only observation").unwrap();
        let metadata = WorkspaceMetadata::capture(&workspace.0).unwrap();

        {
            let mut database = Database::open(&path.0).unwrap();
            database
                .create_session_with_workspace("session-1", metadata, 10)
                .unwrap();
            let session = database
                .observe_workspace_file("session-1", "src/main.rs", 11)
                .unwrap();
            assert_eq!(session.workspace_files.len(), 1);
            assert_eq!(
                session.workspace_files[0].relative_path(),
                PathBuf::from("src/main.rs")
            );
        }

        let mut database = Database::open(&path.0).unwrap();
        database.validate_workspace_files("session-1").unwrap();
        let loaded = database.load_session("session-1").unwrap();
        assert_eq!(loaded.workspace_files.len(), 1);
        assert!(matches!(
            database.session_events("session-1").unwrap().last(),
            Some(StoredSessionEvent::WorkspaceFileObserved { .. })
        ));
        let rebuilt = database.rebuild_snapshot("session-1").unwrap();
        assert_eq!(rebuilt.workspace_files, loaded.workspace_files);

        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            database.validate_workspace_files("session-1"),
            Err(SessionStoreError::WorkspaceFileMetadata(
                WorkspaceFileMetadataError::Changed(_)
            ))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn should_not_record_file_observations_for_sessions_without_a_workspace() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();

        assert!(matches!(
            database.observe_workspace_file("session-1", "src/main.rs", 11),
            Err(SessionStoreError::WorkspaceNotConfigured)
        ));
        assert!(matches!(
            database.validate_workspace_files("session-1"),
            Err(SessionStoreError::WorkspaceNotConfigured)
        ));
        assert_eq!(
            database.load_session("session-1").unwrap().event_sequence,
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_persist_only_the_hash_for_an_explicitly_hashed_workspace_file() {
        let path = TestDatabase::new();
        let workspace = TestWorkspace::new();
        fs::write(workspace.0.join("source.txt"), b"local bytes stay local").unwrap();
        let metadata = WorkspaceMetadata::capture(&workspace.0).unwrap();
        {
            let mut database = Database::open(&path.0).unwrap();
            database
                .create_session_with_workspace("session-1", metadata, 10)
                .unwrap();
            let session = database
                .observe_workspace_file_with_hash("session-1", "source.txt", 11)
                .unwrap();
            assert_eq!(
                session.workspace_files[0].content_sha256(),
                Some("bc7149276653552cf844e69f831e43200135b29dd7f9401c6d0910b65665b08c")
            );
        }

        let database = Database::open(&path.0).unwrap();
        database.validate_workspace_files("session-1").unwrap();
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.workspace_files.len(), 1);
        assert_eq!(
            session.workspace_files[0].content_sha256(),
            Some("bc7149276653552cf844e69f831e43200135b29dd7f9401c6d0910b65665b08c")
        );
        let serialized = serde_json::to_string(&session.workspace_files[0]).unwrap();
        assert!(!serialized.contains("local bytes stay local"));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_to_create_a_session_from_stale_workspace_identity() {
        let path = TestDatabase::new();
        let workspace = TestWorkspace::new();
        let moved_workspace = workspace.0.with_extension("moved");
        let metadata = WorkspaceMetadata::capture(&workspace.0).unwrap();
        fs::rename(&workspace.0, &moved_workspace).unwrap();
        fs::create_dir(&workspace.0).unwrap();

        let mut database = Database::open(&path.0).unwrap();
        assert!(matches!(
            database.create_session_with_workspace("session-1", metadata, 10),
            Err(SessionStoreError::WorkspaceMetadata(
                WorkspaceMetadataError::IdentityChanged(_)
            ))
        ));
        assert!(database.list_sessions().unwrap().is_empty());
        fs::remove_dir_all(moved_workspace).unwrap();
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
    fn should_rollback_event_and_projection_when_snapshot_write_fails() {
        let path = TestDatabase::new();
        {
            let mut database = Database::open(&path.0).unwrap();
            database.create_session("session-1", 10).unwrap();
            database
                .connection
                .execute_batch(
                    "CREATE TRIGGER reject_snapshot_update BEFORE UPDATE ON session_snapshots
                     BEGIN SELECT RAISE(ABORT, 'injected snapshot failure'); END;",
                )
                .unwrap();

            assert!(matches!(
                database.enqueue_message("session-1", message("m1"), 11),
                Err(SessionStoreError::Sqlite(_))
            ));
        }

        let database = Database::open(&path.0).unwrap();
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.event_sequence, 1);
        assert!(session.queued_messages.is_empty());
        assert_eq!(session.updated_at_ms, 10);
        assert_eq!(
            database.session_events("session-1").unwrap(),
            vec![StoredSessionEvent::Created {
                session_id: "session-1".into(),
                workspace: None,
            }]
        );
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

    #[test]
    fn should_recover_active_work_paused_without_requeueing_or_repeating_it() {
        let path = TestDatabase::new();
        {
            let mut database = Database::open(&path.0).unwrap();
            database.create_session("session-1", 10).unwrap();
            database
                .enqueue_message("session-1", message("m1"), 11)
                .unwrap();
            database
                .enqueue_message("session-1", message("m2"), 12)
                .unwrap();
            let started = database
                .start_next_message("session-1", 13)
                .unwrap()
                .unwrap();
            assert_eq!(started.status, StoredSessionStatus::Active);
            assert_eq!(started.active_message, Some(message("m1")));
            assert_eq!(started.queued_messages, vec![message("m2")]);
        }

        let mut database = Database::open(&path.0).unwrap();
        let before_recovery = database.load_session("session-1").unwrap();
        assert_eq!(before_recovery.status, StoredSessionStatus::Active);
        assert_eq!(before_recovery.active_message, Some(message("m1")));

        let recovered = database.recover_active_sessions(20).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, StoredSessionStatus::Paused);
        assert_eq!(recovered[0].active_message, Some(message("m1")));
        assert_eq!(recovered[0].queued_messages, vec![message("m2")]);
        assert!(recovered[0].recovery_needs_revalidation);
        assert!(recovered[0].active_work_uncertain);

        assert!(database.recover_active_sessions(21).unwrap().is_empty());
        let after_second_recovery = database.load_session("session-1").unwrap();
        assert_eq!(
            after_second_recovery.event_sequence,
            recovered[0].event_sequence
        );
        assert_eq!(
            database.session_events("session-1").unwrap().last(),
            Some(&StoredSessionEvent::RecoveredPaused {
                active_message_id: Some("m1".into())
            })
        );
    }

    #[test]
    fn should_recover_once_when_two_connections_race_after_restart() {
        let path = TestDatabase::new();
        {
            let mut database = Database::open(&path.0).unwrap();
            database.create_session("session-1", 10).unwrap();
            database
                .enqueue_message("session-1", message("m1"), 11)
                .unwrap();
            database.start_next_message("session-1", 12).unwrap();
        }

        let barrier = Arc::new(Barrier::new(2));
        let first_path = path.0.clone();
        let first_barrier = Arc::clone(&barrier);
        let second_path = path.0.clone();
        let second_barrier = Arc::clone(&barrier);
        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(move || {
                let mut database = Database::open(first_path).unwrap();
                first_barrier.wait();
                database.recover_active_sessions(20).unwrap().len()
            });
            let second = scope.spawn(move || {
                let mut database = Database::open(second_path).unwrap();
                second_barrier.wait();
                database.recover_active_sessions(20).unwrap().len()
            });
            (first.join().unwrap(), second.join().unwrap())
        });

        assert_eq!(first + second, 1);
        let database = Database::open(&path.0).unwrap();
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.status, StoredSessionStatus::Paused);
        assert_eq!(session.active_message, Some(message("m1")));
        assert!(session.recovery_needs_revalidation);
        assert!(session.active_work_uncertain);
        assert_eq!(
            database
                .session_events("session-1")
                .unwrap()
                .iter()
                .filter(|event| matches!(event, StoredSessionEvent::RecoveredPaused { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn should_not_advance_paused_or_active_sessions_implicitly() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        assert!(
            database
                .start_next_message("session-1", 11)
                .unwrap()
                .is_none()
        );
        database
            .enqueue_message("session-1", message("m1"), 12)
            .unwrap();
        database
            .enqueue_message("session-1", message("m2"), 13)
            .unwrap();
        database.start_next_message("session-1", 14).unwrap();
        assert!(matches!(
            database.start_next_message("session-1", 15),
            Err(SessionStoreError::InvalidTransition)
        ));

        let completed = database
            .complete_active_message("session-1", Outcome::Success, 16)
            .unwrap();
        assert_eq!(completed.status, StoredSessionStatus::Paused);
        assert_eq!(completed.active_message, None);
        assert_eq!(completed.queued_messages, vec![message("m2")]);
        assert!(matches!(
            database.enqueue_message("session-1", message("m1"), 17),
            Err(SessionStoreError::DuplicateMessageId)
        ));
        assert_eq!(database.load_session("session-1").unwrap(), completed);
    }

    #[test]
    fn should_keep_an_interrupted_message_paused_until_explicit_recovery_review() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        database
            .enqueue_message("session-1", message("m1"), 11)
            .unwrap();
        database.start_next_message("session-1", 12).unwrap();
        let paused = database.pause_session("session-1", 13).unwrap();
        assert_eq!(paused.status, StoredSessionStatus::Paused);
        assert_eq!(paused.active_message, Some(message("m1")));
        assert!(!paused.active_work_uncertain);
        assert!(matches!(
            database.complete_active_message("session-1", Outcome::Success, 14),
            Err(SessionStoreError::InvalidTransition)
        ));
        assert_eq!(database.load_session("session-1").unwrap(), paused);
    }

    #[test]
    fn should_not_advance_a_recovered_session_before_revalidation() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        database
            .enqueue_message("session-1", message("m1"), 11)
            .unwrap();
        database
            .enqueue_message("session-1", message("m2"), 12)
            .unwrap();
        database.start_next_message("session-1", 13).unwrap();
        let recovered = database.recover_active_sessions(14).unwrap();
        let sequence_before_attempt = recovered[0].event_sequence;

        assert!(matches!(
            database.start_next_message("session-1", 15),
            Err(SessionStoreError::RevalidationRequired)
        ));

        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.status, StoredSessionStatus::Paused);
        assert_eq!(session.active_message, Some(message("m1")));
        assert_eq!(session.queued_messages, vec![message("m2")]);
        assert!(session.recovery_needs_revalidation);
        assert!(session.active_work_uncertain);
        assert_eq!(session.event_sequence, sequence_before_attempt);
    }

    #[test]
    fn should_list_sessions_consistently_and_detect_active_sessions() {
        let path = TestDatabase::new();
        let mut registry = SessionRegistry::open(&path.0).unwrap();
        registry.create("older", 10).unwrap();
        registry.enqueue("older", message("m1"), 11).unwrap();
        registry.create("newer", 12).unwrap();
        registry.start_next("older", 13).unwrap();

        let listed = registry.list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["older", "newer"]
        );
        let active = registry.active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "older");
        assert_eq!(active[0].active_message, Some(message("m1")));
    }

    #[test]
    fn should_read_bounded_contiguous_event_pages_after_a_valid_sequence_cursor() {
        let path = TestDatabase::new();
        let mut registry = SessionRegistry::open(&path.0).unwrap();
        registry.create("session-1", 10).unwrap();
        registry.enqueue("session-1", message("m1"), 11).unwrap();
        registry.enqueue("session-1", message("m2"), 12).unwrap();
        registry.enqueue("session-1", message("m3"), 13).unwrap();

        let (first_page, first_cursor, first_has_more) =
            registry.session_events_after("session-1", 0, 2).unwrap();
        assert_eq!(
            first_page
                .iter()
                .map(|record| record.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(first_page[0].occurred_at_ms, 10);
        assert!(matches!(
            first_page[1].event,
            StoredSessionEvent::MessageQueued { .. }
        ));
        assert_eq!(first_cursor, 2);
        assert!(first_has_more);

        let (second_page, second_cursor, second_has_more) = registry
            .session_events_after("session-1", first_cursor, 2)
            .unwrap();
        assert_eq!(
            second_page
                .iter()
                .map(|record| record.sequence)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(second_cursor, 4);
        assert!(!second_has_more);
        assert!(registry.session_events_after("session-1", 5, 1).is_err());
        assert!(registry.session_events_after("session-1", 0, 0).is_err());
    }

    #[test]
    fn should_serialize_concurrent_duplicate_enqueues_across_real_connections() {
        let path = TestDatabase::new();
        let mut database = Database::open(&path.0).unwrap();
        database.create_session("session-1", 10).unwrap();
        drop(database);

        let barrier = Arc::new(Barrier::new(2));
        let first_path = path.0.clone();
        let first_barrier = Arc::clone(&barrier);
        let second_path = path.0.clone();
        let second_barrier = Arc::clone(&barrier);
        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(move || {
                let mut database = Database::open(first_path).unwrap();
                first_barrier.wait();
                match database.enqueue_message("session-1", message("same"), 20) {
                    Ok(_) => "inserted",
                    Err(SessionStoreError::DuplicateMessageId) => "duplicate",
                    Err(error) => panic!("unexpected concurrent enqueue error: {error}"),
                }
            });
            let second = scope.spawn(move || {
                let mut database = Database::open(second_path).unwrap();
                second_barrier.wait();
                match database.enqueue_message("session-1", message("same"), 20) {
                    Ok(_) => "inserted",
                    Err(SessionStoreError::DuplicateMessageId) => "duplicate",
                    Err(error) => panic!("unexpected concurrent enqueue error: {error}"),
                }
            });
            (first.join().unwrap(), second.join().unwrap())
        });

        assert_ne!(first, second);
        assert_eq!(
            [first, second]
                .into_iter()
                .filter(|result| *result == "inserted")
                .count(),
            1
        );
        let database = Database::open(&path.0).unwrap();
        let session = database.load_session("session-1").unwrap();
        assert_eq!(session.queued_messages, vec![message("same")]);
        assert_eq!(session.event_sequence, 2);
    }
}
