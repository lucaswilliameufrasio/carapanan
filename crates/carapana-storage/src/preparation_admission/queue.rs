//! Durable capacity intents only. No command, payload, approval or automatic execution is queued.

use super::*;
use rusqlite::Connection;

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preparation_queue_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    max_waiting INTEGER NOT NULL CHECK(max_waiting > 0 AND max_waiting <= 1024),
    per_workspace INTEGER NOT NULL CHECK(per_workspace > 0 AND per_workspace <= max_waiting),
    wait_ms INTEGER NOT NULL CHECK(wait_ms > 0),
    next_turn INTEGER NOT NULL CHECK(next_turn > 0)
) STRICT;
CREATE TABLE IF NOT EXISTS preparation_queue_workspaces (
    workspace_key BLOB PRIMARY KEY CHECK(length(workspace_key) = 16),
    turn INTEGER NOT NULL UNIQUE CHECK(turn > 0)
) STRICT;
CREATE TABLE IF NOT EXISTS preparation_queue_requests (
    request_id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_key BLOB NOT NULL REFERENCES preparation_queue_workspaces(workspace_key),
    reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes > 0),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > created_at_ms)
) STRICT;
CREATE INDEX IF NOT EXISTS preparation_queue_workspace ON preparation_queue_requests(workspace_key, request_id);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationQueuePolicy {
    max_waiting: u32,
    per_workspace: u32,
    wait_ms: i64,
}

impl PreparationQueuePolicy {
    pub fn new(
        max_waiting: u32,
        per_workspace: u32,
        wait: std::time::Duration,
    ) -> Result<Self, PreparationAdmissionError> {
        let wait_ms = i64::try_from(wait.as_millis())
            .map_err(|_| PreparationAdmissionError::InvalidPolicy)?;
        if max_waiting == 0
            || max_waiting > 1024
            || per_workspace == 0
            || per_workspace > max_waiting
            || wait_ms == 0
        {
            return Err(PreparationAdmissionError::InvalidPolicy);
        }
        Ok(Self {
            max_waiting,
            per_workspace,
            wait_ms,
        })
    }
}

/// A capacity intent, NOT an execution capability. Reopening the catalog never starts work.
pub struct PreparationQueueTicket {
    pub(super) id: i64,
    workspace_key: [u8; 16],
    bytes: u64,
    expires_at_ms: i64,
}

impl PreparationQueueTicket {
    pub fn id(&self) -> i64 {
        self.id
    }
    pub fn reserved_bytes(&self) -> u64 {
        self.bytes
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }
}

impl PreparationAdmissionStore {
    /// Explicit opt-in. Once configured, immediate admission cannot bypass waiting contenders.
    pub fn configure_queue(
        &mut self,
        policy: PreparationQueuePolicy,
    ) -> Result<(), PreparationAdmissionError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        transaction.execute(
            "INSERT OR IGNORE INTO preparation_queue_policy VALUES(1, ?1, ?2, ?3, 1)",
            rusqlite::params![policy.max_waiting, policy.per_workspace, policy.wait_ms],
        )?;
        if read_policy(&transaction)? != Some(policy) {
            return Err(PreparationAdmissionError::PolicyMismatch);
        }
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)
    }

    pub fn enqueue_preparation(
        &mut self,
        workspace: &WorkspaceDirectoryIdentity,
        bytes: u64,
        now_ms: i64,
    ) -> Result<PreparationQueueTicket, PreparationAdmissionError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        let policy = read_policy(&transaction)?.ok_or(PreparationAdmissionError::InvalidPolicy)?;
        if now_ms < 0 || bytes == 0 || bytes > self.policy.workspace_max_bytes {
            return Err(PreparationAdmissionError::InvalidRequest);
        }
        let expires_at_ms = now_ms
            .checked_add(policy.wait_ms)
            .ok_or(PreparationAdmissionError::InvalidRequest)?;
        expire(&transaction, now_ms)?;
        let key = workspace.admission_key();
        let (total, own): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(workspace_key = ?1), 0) FROM preparation_queue_requests",
            [key.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if total >= i64::from(policy.max_waiting) || own >= i64::from(policy.per_workspace) {
            return Err(PreparationAdmissionError::QueueFull);
        }
        if own == 0 {
            let turn = next_turn(&transaction)?;
            transaction.execute(
                "INSERT INTO preparation_queue_workspaces VALUES(?1, ?2)",
                rusqlite::params![key.as_slice(), turn],
            )?;
        }
        transaction.execute("INSERT INTO preparation_queue_requests(workspace_key, reserved_bytes, created_at_ms, expires_at_ms) VALUES(?1, ?2, ?3, ?4)",
            rusqlite::params![key.as_slice(), bytes as i64, now_ms, expires_at_ms])?;
        let id = transaction.last_insert_rowid();
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(PreparationQueueTicket {
            id,
            workspace_key: key,
            bytes,
            expires_at_ms,
        })
    }

    /// Bounded metadata recovery and expiry only; no reservation, approval replay or process start.
    pub fn pending_preparations(
        &mut self,
        now_ms: i64,
    ) -> Result<Vec<PreparationQueueTicket>, PreparationAdmissionError> {
        if now_ms < 0 {
            return Err(PreparationAdmissionError::InvalidRequest);
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        let tickets = {
            let policy =
                read_policy(&transaction)?.ok_or(PreparationAdmissionError::InvalidPolicy)?;
            expire(&transaction, now_ms)?;
            let mut statement = transaction.prepare("SELECT r.request_id, r.workspace_key, r.reserved_bytes, r.expires_at_ms FROM preparation_queue_requests r JOIN preparation_queue_workspaces w USING(workspace_key) ORDER BY w.turn, r.request_id LIMIT 1025")?;
            let mut rows = statement.query([])?;
            let mut tickets = Vec::new();
            while let Some(row) = rows.next()? {
                if tickets.len() >= policy.max_waiting as usize {
                    return Err(PreparationAdmissionError::InvalidDatabaseState);
                }
                let key: Vec<u8> = row.get(1)?;
                let bytes: i64 = row.get(2)?;
                tickets.push(PreparationQueueTicket {
                    id: row.get(0)?,
                    workspace_key: key
                        .try_into()
                        .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?,
                    bytes: u64::try_from(bytes)
                        .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?,
                    expires_at_ms: row.get(3)?,
                });
            }
            tickets
        };
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(tickets)
    }

    pub fn cancel_preparation(
        &mut self,
        ticket: &PreparationQueueTicket,
    ) -> Result<bool, PreparationAdmissionError> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_policy(&transaction, self.policy)?;
        let removed = transaction.execute("DELETE FROM preparation_queue_requests WHERE request_id = ?1 AND workspace_key = ?2 AND reserved_bytes = ?3",
            rusqlite::params![ticket.id, ticket.workspace_key.as_slice(), ticket.bytes as i64])? == 1;
        remove_empty_workspaces(&transaction)?;
        transaction
            .commit()
            .map_err(|_| PreparationAdmissionError::CommitOutcomeUncertain)?;
        Ok(removed)
    }
}

fn read_policy(
    connection: &Connection,
) -> Result<Option<PreparationQueuePolicy>, PreparationAdmissionError> {
    let fields: Option<(u32, u32, i64)> = connection.query_row(
        "SELECT max_waiting, per_workspace, wait_ms FROM preparation_queue_policy WHERE singleton = 1", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((max_waiting, per_workspace, wait_ms)) = fields else {
        return Ok(None);
    };
    if wait_ms <= 0 {
        return Err(PreparationAdmissionError::InvalidDatabaseState);
    }
    let policy = PreparationQueuePolicy::new(
        max_waiting,
        per_workspace,
        std::time::Duration::from_millis(wait_ms as u64),
    )?;
    Ok(Some(policy))
}

pub(super) fn enabled(connection: &Connection) -> Result<bool, PreparationAdmissionError> {
    Ok(read_policy(connection)?.is_some())
}

fn next_turn(connection: &Connection) -> Result<i64, PreparationAdmissionError> {
    let turn: i64 = connection.query_row(
        "SELECT next_turn FROM preparation_queue_policy WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let next = turn
        .checked_add(1)
        .filter(|next| *next > 1)
        .ok_or(PreparationAdmissionError::InvalidDatabaseState)?;
    connection.execute(
        "UPDATE preparation_queue_policy SET next_turn = ?1 WHERE singleton = 1",
        [next],
    )?;
    Ok(turn)
}

fn remove_empty_workspaces(connection: &Connection) -> Result<(), PreparationAdmissionError> {
    connection.execute("DELETE FROM preparation_queue_workspaces WHERE NOT EXISTS(SELECT 1 FROM preparation_queue_requests r WHERE r.workspace_key = preparation_queue_workspaces.workspace_key)", [])?;
    Ok(())
}

fn expire(connection: &Connection, now_ms: i64) -> Result<(), PreparationAdmissionError> {
    connection.execute(
        "DELETE FROM preparation_queue_requests WHERE expires_at_ms <= ?1",
        [now_ms],
    )?;
    remove_empty_workspaces(connection)
}

pub(super) fn validate_turn(
    connection: &Connection,
    policy: PreparationAdmissionPolicy,
    ticket: &PreparationQueueTicket,
    workspace: Option<[u8; 16]>,
    bytes: u64,
    now_ms: i64,
) -> Result<(), PreparationAdmissionError> {
    read_policy(connection)?.ok_or(PreparationAdmissionError::InvalidPolicy)?;
    if now_ms >= ticket.expires_at_ms {
        return Err(PreparationAdmissionError::QueueExpired);
    }
    if workspace != Some(ticket.workspace_key) || bytes != ticket.bytes {
        return Err(PreparationAdmissionError::QueueNotReady);
    }
    expire(connection, now_ms)?;
    let matching: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM preparation_queue_requests WHERE request_id = ?1 AND workspace_key = ?2 AND reserved_bytes = ?3 AND expires_at_ms = ?4 AND created_at_ms <= ?5)",
        rusqlite::params![ticket.id, ticket.workspace_key.as_slice(), bytes as i64, ticket.expires_at_ms, now_ms], |row| row.get(0))?;
    if !matching {
        return Err(PreparationAdmissionError::QueueNotReady);
    }
    let usage = read_usage(connection)?;
    if count(usage)? >= policy.max_leases {
        return Err(PreparationAdmissionError::QueueNotReady);
    }
    let mut statement = connection.prepare("SELECT r.request_id, r.workspace_key, r.reserved_bytes FROM preparation_queue_requests r JOIN preparation_queue_workspaces w USING(workspace_key) WHERE r.request_id = (SELECT MIN(q.request_id) FROM preparation_queue_requests q WHERE q.workspace_key = r.workspace_key) ORDER BY w.turn LIMIT 1025")?;
    let mut rows = statement.query([])?;
    let mut inspected = 0;
    while let Some(row) = rows.next()? {
        inspected += 1;
        if inspected > 1024 {
            return Err(PreparationAdmissionError::InvalidDatabaseState);
        }
        let key: Vec<u8> = row.get(1)?;
        let key: [u8; 16] = key
            .try_into()
            .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?;
        let requested: i64 = row.get(2)?;
        let requested = u64::try_from(requested)
            .map_err(|_| PreparationAdmissionError::InvalidDatabaseState)?;
        let own = read_workspace_usage(connection, &key)?;
        if usage
            .reserved_bytes
            .checked_add(requested)
            .is_some_and(|total| total <= policy.max_bytes)
            && own
                .reserved_bytes
                .checked_add(requested)
                .is_some_and(|total| total <= policy.workspace_max_bytes)
            && count(own)? < policy.workspace_max_leases
        {
            let first_id: i64 = row.get(0)?;
            if first_id == ticket.id {
                return Ok(());
            }
            return Err(PreparationAdmissionError::QueueNotReady);
        }
    }
    Err(PreparationAdmissionError::QueueNotReady)
}

fn count(usage: PreparationAdmissionUsage) -> Result<u32, PreparationAdmissionError> {
    usage
        .held_leases
        .checked_add(usage.quarantined_leases)
        .ok_or(PreparationAdmissionError::InvalidDatabaseState)
}

pub(super) fn consume(
    connection: &Connection,
    ticket: &PreparationQueueTicket,
) -> Result<(), PreparationAdmissionError> {
    if connection.execute(
        "DELETE FROM preparation_queue_requests WHERE request_id = ?1",
        [ticket.id],
    )? != 1
    {
        return Err(PreparationAdmissionError::InvalidDatabaseState);
    }
    let waiting: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM preparation_queue_requests WHERE workspace_key = ?1)",
        [ticket.workspace_key.as_slice()],
        |row| row.get(0),
    )?;
    if waiting {
        let turn = next_turn(connection)?;
        connection.execute(
            "UPDATE preparation_queue_workspaces SET turn = ?1 WHERE workspace_key = ?2",
            rusqlite::params![turn, ticket.workspace_key.as_slice()],
        )?;
    } else {
        remove_empty_workspaces(connection)?;
    }
    Ok(())
}
