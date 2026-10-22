//! Safe startup orchestration for the future local daemon.
//!
//! This crate provides local session inspection/attachment IPC, but no provider or tool runner.

#[cfg(unix)]
mod ipc;
#[cfg(unix)]
mod process;
pub mod process_output;

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::sync::atomic::AtomicBool;

use std::time::{SystemTime, UNIX_EPOCH};

use carapana_protocol::{
    AttentionItem, AttentionReason, DaemonErrorCode, DaemonRequest, DaemonResponse,
    DaemonSessionEvent, Envelope, MAX_QUEUED_MESSAGE_BYTES, MAX_SELECTION_FIELD_BYTES,
    MAX_WORKSPACE_PATH_BYTES, MessageOrigin, QueuedMessage, Selection, SessionEventBatch,
    SessionEventRecord, SessionSnapshot, SessionStatus, SessionSummary, WorkspaceFileReview,
    WorkspaceFileReviewItem, WorkspaceFileReviewStatus,
};
use carapana_storage::{
    MigrationError, PersistedSession, SessionRegistry, SessionStoreError, StoredSessionEvent,
    StoredSessionStatus, UserDatabaseError, WorkspaceFileValidationStatus, WorkspaceMetadata,
};

#[cfg(unix)]
pub use ipc::{IpcAttachment, IpcError, IpcServer, request as ipc_request};
#[cfg(unix)]
pub use process::{
    CapturedProcessEnd, OwnedProcessInfo, OwnedProcessOutput, ProcessCaptureSettings, ProcessError,
    ProcessShutdownPolicy, ProcessSpec, ProcessSupervisor, run_process_host,
};
#[cfg(target_os = "linux")]
pub use process::{
    SandboxExecutionError, SandboxExecutionLimits, SandboxExecutionProposal, SandboxExecutionScope,
};

static GENERATED_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum DaemonStartupError {
    Migration(MigrationError),
    SessionStore(SessionStoreError),
    UserDatabase(UserDatabaseError),
}

impl fmt::Display for DaemonStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Migration(error) => write!(formatter, "daemon storage could not open: {error}"),
            Self::SessionStore(error) => {
                write!(formatter, "daemon session recovery failed: {error}")
            }
            Self::UserDatabase(error) => write!(formatter, "daemon user database failed: {error}"),
        }
    }
}

impl Error for DaemonStartupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Migration(error) => Some(error),
            Self::SessionStore(error) => Some(error),
            Self::UserDatabase(error) => Some(error),
        }
    }
}

/// Session storage after startup recovery has completed.
///
/// The registry is not exposed until all previously active sessions have been
/// converted to paused recovery state. Uncertain work is preserved, not resumed.
pub struct DaemonRuntime {
    registry: SessionRegistry,
    startup_recovered: Vec<PersistedSession>,
    attached_clients: HashMap<String, u64>,
}

impl DaemonRuntime {
    /// Open the secured per-user database, recover interrupted sessions, then return.
    pub fn open_user(now_ms: i64) -> Result<Self, DaemonStartupError> {
        let registry = SessionRegistry::open_user()?;
        Self::recover(registry, now_ms)
    }

    /// Open a caller-selected database; primarily useful for tests and explicit embedding.
    pub fn open(path: impl AsRef<Path>, now_ms: i64) -> Result<Self, DaemonStartupError> {
        let registry = SessionRegistry::open(path)?;
        Self::recover(registry, now_ms)
    }

    /// Sessions transitioned to paused state during this process's startup.
    pub fn startup_recovered(&self) -> &[PersistedSession] {
        &self.startup_recovered
    }

    /// List sessions after startup recovery has completed.
    pub fn sessions(&mut self) -> Result<Vec<PersistedSession>, SessionStoreError> {
        self.registry.list()
    }

    /// Handle a one-shot request. Attach/detach are handled by persistent IPC connections.
    pub fn handle(&mut self, request: Envelope<DaemonRequest>) -> Envelope<DaemonResponse> {
        let response = match request.payload {
            DaemonRequest::ListSessions {} => match self.sessions() {
                Ok(sessions) => DaemonResponse::Sessions {
                    sessions: sessions
                        .into_iter()
                        .map(|session| self.session_summary(session))
                        .collect(),
                },
                Err(_) => DaemonResponse::Error {
                    code: DaemonErrorCode::StorageUnavailable,
                },
            },
            DaemonRequest::CreateSession { workspace_path } => {
                match self.create_session(&workspace_path) {
                    Ok(session_id) => DaemonResponse::SessionCreated { session_id },
                    Err(code) => DaemonResponse::Error { code },
                }
            }
            DaemonRequest::EnqueueMessage {
                session_id,
                text,
                selection,
                origin,
            } => match self.enqueue_message(&session_id, text, selection, origin) {
                Ok(message_id) => DaemonResponse::MessageQueued {
                    session_id,
                    message_id,
                },
                Err(code) => DaemonResponse::Error { code },
            },
            DaemonRequest::HibernateSession { session_id } => {
                match self.hibernate_session(&session_id) {
                    Ok(()) => DaemonResponse::SessionHibernated { session_id },
                    Err(code) => DaemonResponse::Error { code },
                }
            }
            DaemonRequest::WakeSession { session_id } => match self.wake_session(&session_id) {
                Ok(()) => DaemonResponse::SessionWoken { session_id },
                Err(code) => DaemonResponse::Error { code },
            },
            DaemonRequest::ListAttention {} => match self.attention_queue() {
                Ok(items) => DaemonResponse::Attention { items },
                Err(_) => DaemonResponse::Error {
                    code: DaemonErrorCode::StorageUnavailable,
                },
            },
            DaemonRequest::ReviewWorkspace { session_id } => {
                match self.review_workspace(&session_id) {
                    Ok(review) => DaemonResponse::WorkspaceReview { review },
                    Err(SessionStoreError::SessionNotFound) => DaemonResponse::Error {
                        code: DaemonErrorCode::SessionNotFound,
                    },
                    Err(_) => DaemonResponse::Error {
                        code: DaemonErrorCode::WorkspaceReviewUnavailable,
                    },
                }
            }
            DaemonRequest::AcknowledgeRecoveryPath { session_id, path } => {
                match self.acknowledge_recovery_path(&session_id, &path) {
                    Ok((path, status)) => DaemonResponse::RecoveryPathAcknowledged { path, status },
                    Err(code) => DaemonResponse::Error { code },
                }
            }
            DaemonRequest::ResumeRecovered { session_id } => {
                match self.resume_recovered_session(&session_id) {
                    Ok(()) => DaemonResponse::RecoveryResumed { session_id },
                    Err(code) => DaemonResponse::Error { code },
                }
            }
            DaemonRequest::Attach { .. } | DaemonRequest::Detach {} => DaemonResponse::Error {
                code: DaemonErrorCode::ConnectionRequired,
            },
            DaemonRequest::EventsAfter { .. } => DaemonResponse::Error {
                code: DaemonErrorCode::ConnectionRequired,
            },
        };
        Envelope::new(response)
    }

    fn create_session(&mut self, workspace_path: &str) -> Result<String, DaemonErrorCode> {
        if workspace_path.len() > MAX_WORKSPACE_PATH_BYTES {
            return Err(DaemonErrorCode::WorkspaceCreationRejected);
        }
        let workspace = WorkspaceMetadata::capture(workspace_path)
            .map_err(|_| DaemonErrorCode::WorkspaceCreationRejected)?;
        let session_id = generated_id("session")?;
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        self.registry
            .create_with_workspace(&session_id, workspace, now_ms)
            .map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        Ok(session_id)
    }

    fn enqueue_message(
        &mut self,
        session_id: &str,
        text: String,
        selection: Selection,
        origin: MessageOrigin,
    ) -> Result<String, DaemonErrorCode> {
        if text.trim().is_empty()
            || text.len() > MAX_QUEUED_MESSAGE_BYTES
            || !valid_selection_field(&selection.profile)
            || !valid_selection_field(&selection.provider)
            || !valid_selection_field(&selection.model)
            || !valid_selection_field(&selection.variant)
        {
            return Err(DaemonErrorCode::InvalidQueuedMessage);
        }

        let message_id = generated_id("message")?;
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        let message = QueuedMessage {
            id: message_id.clone(),
            text,
            origin: origin.as_str().to_owned(),
            selection,
        };
        self.registry
            .enqueue(session_id, message, now_ms)
            .map_err(|error| match error {
                SessionStoreError::SessionNotFound => DaemonErrorCode::SessionNotFound,
                _ => DaemonErrorCode::StorageUnavailable,
            })?;
        Ok(message_id)
    }

    fn hibernate_session(&mut self, session_id: &str) -> Result<(), DaemonErrorCode> {
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        self.registry
            .hibernate(session_id, now_ms)
            .map(|_| ())
            .map_err(|error| match error {
                SessionStoreError::SessionNotFound => DaemonErrorCode::SessionNotFound,
                SessionStoreError::InvalidTransition => DaemonErrorCode::HibernationBlocked,
                _ => DaemonErrorCode::StorageUnavailable,
            })
    }

    fn wake_session(&mut self, session_id: &str) -> Result<(), DaemonErrorCode> {
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        self.registry
            .wake_hibernated(session_id, now_ms)
            .map(|_| ())
            .map_err(|error| match error {
                SessionStoreError::SessionNotFound => DaemonErrorCode::SessionNotFound,
                SessionStoreError::InvalidTransition => DaemonErrorCode::SessionNotHibernated,
                _ => DaemonErrorCode::StorageUnavailable,
            })
    }

    /// Derive a stable attention list from durable recovery state; no separate
    /// acknowledgement or dismiss state can diverge from the session event log.
    pub fn attention_queue(&mut self) -> Result<Vec<AttentionItem>, SessionStoreError> {
        let mut items = self
            .registry
            .list()?
            .into_iter()
            .filter(|session| session.recovery_needs_revalidation || session.active_work_uncertain)
            .map(|session| AttentionItem {
                session_id: session.session_id,
                reason: AttentionReason::RecoveryReview,
                active_work_uncertain: session.active_work_uncertain,
                updated_at_ms: session.updated_at_ms,
                event_sequence: session.event_sequence,
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.updated_at_ms
                .cmp(&right.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(items)
    }

    fn review_workspace(&self, session_id: &str) -> Result<WorkspaceFileReview, SessionStoreError> {
        let report = self.registry.review_workspace_files(session_id)?;
        let files = report
            .files
            .into_iter()
            .map(|file| WorkspaceFileReviewItem {
                path: file.relative_path.to_string_lossy().into_owned(),
                status: workspace_review_status(file.status),
            })
            .collect();
        Ok(WorkspaceFileReview { files })
    }

    fn acknowledge_recovery_path(
        &mut self,
        session_id: &str,
        path: &str,
    ) -> Result<(String, WorkspaceFileReviewStatus), DaemonErrorCode> {
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        self.registry
            .acknowledge_recovery_path(session_id, path, now_ms)
            .map(|acknowledgment| {
                (
                    acknowledgment.relative_path.to_string_lossy().into_owned(),
                    workspace_review_status(acknowledgment.status),
                )
            })
            .map_err(map_recovery_error)
    }

    fn resume_recovered_session(&mut self, session_id: &str) -> Result<(), DaemonErrorCode> {
        let now_ms = current_time_ms().map_err(|_| DaemonErrorCode::StorageUnavailable)?;
        self.registry
            .resume_recovered_session(session_id, now_ms)
            .map(|_| ())
            .map_err(map_recovery_error)
    }

    fn events_after(
        &mut self,
        session_id: &str,
        after_sequence: i64,
    ) -> Result<SessionEventBatch, SessionStoreError> {
        const EVENT_PAGE_SIZE: u16 = 16;
        let (records, next_sequence, has_more) =
            self.registry
                .session_events_after(session_id, after_sequence, EVENT_PAGE_SIZE)?;
        let events = records
            .into_iter()
            .filter_map(|record| {
                let event = match record.event {
                    StoredSessionEvent::Created { .. } => DaemonSessionEvent::Created,
                    StoredSessionEvent::WorkspaceFileObserved { .. } => return None,
                    StoredSessionEvent::MessageQueued { message } => {
                        DaemonSessionEvent::MessageQueued { message }
                    }
                    StoredSessionEvent::Hibernated => DaemonSessionEvent::Hibernated,
                    StoredSessionEvent::Woken => DaemonSessionEvent::Woken,
                    StoredSessionEvent::MessageStarted { message_id } => {
                        DaemonSessionEvent::MessageStarted { message_id }
                    }
                    StoredSessionEvent::Paused => DaemonSessionEvent::Paused,
                    StoredSessionEvent::Completed {
                        message_id,
                        outcome,
                    } => DaemonSessionEvent::Completed {
                        message_id,
                        outcome,
                    },
                    StoredSessionEvent::RecoveredPaused { active_message_id } => {
                        DaemonSessionEvent::RecoveredPaused { active_message_id }
                    }
                    StoredSessionEvent::RecoveryPathAcknowledged { acknowledgment } => {
                        DaemonSessionEvent::RecoveryPathAcknowledged {
                            path: acknowledgment
                                .relative_path()
                                .to_string_lossy()
                                .into_owned(),
                            status: workspace_review_status(acknowledgment.status()),
                        }
                    }
                    StoredSessionEvent::RecoveryResumed {
                        active_message_id, ..
                    } => DaemonSessionEvent::RecoveryResumed { active_message_id },
                };
                Some(SessionEventRecord {
                    sequence: record.sequence,
                    occurred_at_ms: record.occurred_at_ms,
                    event,
                })
            })
            .collect();
        Ok(SessionEventBatch {
            events,
            next_sequence,
            has_more,
        })
    }

    fn attach(&mut self, session_id: &str) -> Envelope<DaemonResponse> {
        let session = match self.registry.get(session_id) {
            Ok(session) => session,
            Err(SessionStoreError::SessionNotFound) => {
                return Envelope::new(DaemonResponse::Error {
                    code: DaemonErrorCode::SessionNotFound,
                });
            }
            Err(_) => {
                return Envelope::new(DaemonResponse::Error {
                    code: DaemonErrorCode::StorageUnavailable,
                });
            }
        };
        let attached_clients = self
            .attached_clients
            .entry(session_id.to_owned())
            .or_default();
        *attached_clients = attached_clients.saturating_add(1);
        let snapshot = Self::session_snapshot(session, *attached_clients);
        Envelope::new(DaemonResponse::Attached {
            snapshot: Box::new(snapshot),
        })
    }

    fn detach(&mut self, session_id: &str) -> u64 {
        let Some(attached_clients) = self.attached_clients.get_mut(session_id) else {
            return 0;
        };
        *attached_clients = attached_clients.saturating_sub(1);
        if *attached_clients == 0 {
            self.attached_clients.remove(session_id);
            0
        } else {
            *attached_clients
        }
    }

    fn session_summary(&self, session: PersistedSession) -> SessionSummary {
        SessionSummary {
            session_id: session.session_id.clone(),
            status: Self::session_status(session.status),
            queued_count: u64::try_from(session.queued_messages.len()).unwrap_or(u64::MAX),
            has_active_message: session.active_message.is_some(),
            recovery_needs_revalidation: session.recovery_needs_revalidation,
            active_work_uncertain: session.active_work_uncertain,
            attached_clients: self
                .attached_clients
                .get(&session.session_id)
                .copied()
                .unwrap_or(0),
            updated_at_ms: session.updated_at_ms,
        }
    }

    fn session_snapshot(session: PersistedSession, attached_clients: u64) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session.session_id,
            status: Self::session_status(session.status),
            queued_messages: session.queued_messages,
            active_message: session.active_message,
            recovery_needs_revalidation: session.recovery_needs_revalidation,
            active_work_uncertain: session.active_work_uncertain,
            attached_clients,
            created_at_ms: session.created_at_ms,
            updated_at_ms: session.updated_at_ms,
            event_sequence: session.event_sequence,
        }
    }

    fn session_status(status: StoredSessionStatus) -> SessionStatus {
        match status {
            StoredSessionStatus::Active => SessionStatus::Active,
            StoredSessionStatus::Paused => SessionStatus::Paused,
            StoredSessionStatus::Hibernated => SessionStatus::Hibernated,
        }
    }

    fn recover(mut registry: SessionRegistry, now_ms: i64) -> Result<Self, DaemonStartupError> {
        let startup_recovered = registry.recover_after_restart(now_ms)?;
        Ok(Self {
            registry,
            startup_recovered,
            attached_clients: HashMap::new(),
        })
    }
}

fn workspace_review_status(status: WorkspaceFileValidationStatus) -> WorkspaceFileReviewStatus {
    match status {
        WorkspaceFileValidationStatus::Unchanged => WorkspaceFileReviewStatus::Unchanged,
        WorkspaceFileValidationStatus::Changed => WorkspaceFileReviewStatus::Changed,
        WorkspaceFileValidationStatus::Missing => WorkspaceFileReviewStatus::Missing,
        WorkspaceFileValidationStatus::Unreadable => WorkspaceFileReviewStatus::Unreadable,
        WorkspaceFileValidationStatus::Unsafe => WorkspaceFileReviewStatus::Unsafe,
        WorkspaceFileValidationStatus::TooLarge => WorkspaceFileReviewStatus::TooLarge,
        WorkspaceFileValidationStatus::Unavailable => WorkspaceFileReviewStatus::Unavailable,
    }
}

fn current_time_ms() -> Result<i64, ()> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?;
    i64::try_from(duration.as_millis()).map_err(|_| ())
}

fn generated_id(prefix: &str) -> Result<String, DaemonErrorCode> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DaemonErrorCode::StorageUnavailable)?
        .as_nanos();
    let sequence = GENERATED_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{prefix}-{timestamp:x}-{:x}-{sequence:x}",
        std::process::id()
    ))
}

fn valid_selection_field(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_SELECTION_FIELD_BYTES
}

fn map_recovery_error(error: SessionStoreError) -> DaemonErrorCode {
    match error {
        SessionStoreError::SessionNotFound => DaemonErrorCode::SessionNotFound,
        SessionStoreError::WorkspaceNotConfigured
        | SessionStoreError::WorkspaceMetadata(_)
        | SessionStoreError::WorkspaceFileMetadata(_) => {
            DaemonErrorCode::WorkspaceReviewUnavailable
        }
        SessionStoreError::RecoveryNotPending => DaemonErrorCode::RecoveryNotPending,
        SessionStoreError::RecoveryPathNotAcknowledgable => {
            DaemonErrorCode::RecoveryPathNotAcknowledgable
        }
        SessionStoreError::RecoveryAcknowledgmentRequired => {
            DaemonErrorCode::RecoveryAcknowledgmentRequired
        }
        SessionStoreError::RecoveryAcknowledgmentStale => {
            DaemonErrorCode::RecoveryAcknowledgmentStale
        }
        SessionStoreError::RecoveryBlocked => DaemonErrorCode::RecoveryBlocked,
        _ => DaemonErrorCode::StorageUnavailable,
    }
}

/// Recovered daemon runtime bound to its local IPC listener.
///
/// Construction reserves the socket before paused crash recovery, preventing a second
/// daemon from recovering the same sessions. It accepts no requests until recovery has
/// completed and the host starts serving with its supplied shutdown signal.
#[cfg(unix)]
pub struct DaemonService {
    runtime: DaemonRuntime,
    server: IpcServer,
    process_supervisor: ProcessSupervisor,
}

#[cfg(unix)]
#[derive(Debug)]
pub enum DaemonServiceError {
    Startup(DaemonStartupError),
    Ipc(IpcError),
    Process(ProcessError),
}

#[cfg(unix)]
impl fmt::Display for DaemonServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup(error) => write!(formatter, "daemon service startup failed: {error}"),
            Self::Ipc(error) => write!(formatter, "daemon service IPC failed: {error}"),
            Self::Process(error) => write!(formatter, "daemon process supervision failed: {error}"),
        }
    }
}

#[cfg(unix)]
impl Error for DaemonServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Startup(error) => Some(error),
            Self::Ipc(error) => Some(error),
            Self::Process(error) => Some(error),
        }
    }
}

#[cfg(unix)]
impl DaemonService {
    /// Reserve the current user's private socket, then recover before serving requests.
    pub fn open_user(now_ms: i64) -> Result<Self, DaemonServiceError> {
        let registry = SessionRegistry::open_user().map_err(DaemonStartupError::from)?;
        let server = IpcServer::bind_user()?;
        let runtime = DaemonRuntime::recover(registry, now_ms)?;
        let process_supervisor = ProcessSupervisor::for_current_executable()?;
        Ok(Self {
            runtime,
            server,
            process_supervisor,
        })
    }

    /// Open an explicit database/socket pair, primarily for tests and embedding.
    ///
    /// The socket is reserved before crash recovery, but no connection is accepted until
    /// recovery succeeds and the caller invokes `serve_until`.
    pub fn open(
        database_path: impl AsRef<Path>,
        socket_path: impl AsRef<Path>,
        now_ms: i64,
    ) -> Result<Self, DaemonServiceError> {
        let registry = SessionRegistry::open(database_path).map_err(DaemonStartupError::from)?;
        let server = IpcServer::bind(socket_path)?;
        let runtime = DaemonRuntime::recover(registry, now_ms)?;
        let process_supervisor = ProcessSupervisor::for_current_executable()?;
        Ok(Self {
            runtime,
            server,
            process_supervisor,
        })
    }

    /// Open an explicit database/socket pair and use the supplied executable as the
    /// private process-host entrypoint. Intended for integration tests and embedding.
    pub fn open_with_process_host(
        database_path: impl AsRef<Path>,
        socket_path: impl AsRef<Path>,
        process_host_executable: impl AsRef<Path>,
        now_ms: i64,
    ) -> Result<Self, DaemonServiceError> {
        let registry = SessionRegistry::open(database_path).map_err(DaemonStartupError::from)?;
        let server = IpcServer::bind(socket_path)?;
        let runtime = DaemonRuntime::recover(registry, now_ms)?;
        let process_supervisor = ProcessSupervisor::new(process_host_executable)?;
        Ok(Self {
            runtime,
            server,
            process_supervisor,
        })
    }

    pub fn startup_recovered(&self) -> &[PersistedSession] {
        self.runtime.startup_recovered()
    }

    pub fn socket_path(&self) -> &Path {
        self.server.path()
    }

    /// Start an owned process only for a session persisted by this daemon.
    pub fn spawn_owned_process(
        &mut self,
        session_id: &str,
        spec: ProcessSpec,
    ) -> Result<OwnedProcessInfo, ProcessError> {
        self.verify_process_owner_session(session_id)?;
        self.process_supervisor
            .spawn_owned_process(session_id, spec)
    }

    pub fn owned_processes(&mut self) -> Result<Vec<OwnedProcessInfo>, ProcessError> {
        self.process_supervisor.owned_processes()
    }

    pub fn stop_owned_process(
        &mut self,
        session_id: &str,
        process_id: &str,
    ) -> Result<(), ProcessError> {
        self.verify_process_owner_session(session_id)?;
        self.process_supervisor
            .stop_owned_process(session_id, process_id)
    }

    pub fn restart_owned_process(
        &mut self,
        session_id: &str,
        process_id: &str,
        spec: ProcessSpec,
    ) -> Result<OwnedProcessInfo, ProcessError> {
        self.verify_process_owner_session(session_id)?;
        self.process_supervisor
            .restart_owned_process(session_id, process_id, spec)
    }

    pub fn stop_session_processes(&mut self, session_id: &str) -> Result<(), ProcessError> {
        self.verify_process_owner_session(session_id)?;
        self.process_supervisor.stop_session_processes(session_id)
    }

    pub fn serve_until(&mut self, shutdown: &AtomicBool) -> Result<(), DaemonServiceError> {
        let server_result = self.server.serve_until(&mut self.runtime, shutdown);
        let supervisor_result = self.process_supervisor.shutdown();
        match (server_result, supervisor_result) {
            (Err(error), _) => Err(DaemonServiceError::Ipc(error)),
            (Ok(()), Err(error)) => Err(DaemonServiceError::Process(error)),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn verify_process_owner_session(&self, session_id: &str) -> Result<(), ProcessError> {
        if session_id.trim().is_empty()
            || session_id.len() > 256
            || session_id.chars().any(char::is_control)
        {
            return Err(ProcessError::InvalidSessionId);
        }
        match self.runtime.registry.get(session_id) {
            Ok(_) => Ok(()),
            Err(SessionStoreError::SessionNotFound) => Err(ProcessError::OwnerSessionMissing),
            Err(_) => Err(ProcessError::SessionStorageUnavailable),
        }
    }
}

#[cfg(unix)]
impl Drop for DaemonService {
    fn drop(&mut self) {
        let _ = self.process_supervisor.shutdown();
    }
}

impl From<MigrationError> for DaemonStartupError {
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}

impl From<SessionStoreError> for DaemonStartupError {
    fn from(error: SessionStoreError) -> Self {
        Self::SessionStore(error)
    }
}

impl From<UserDatabaseError> for DaemonStartupError {
    fn from(error: UserDatabaseError) -> Self {
        Self::UserDatabase(error)
    }
}

#[cfg(unix)]
impl From<DaemonStartupError> for DaemonServiceError {
    fn from(error: DaemonStartupError) -> Self {
        Self::Startup(error)
    }
}

#[cfg(unix)]
impl From<IpcError> for DaemonServiceError {
    fn from(error: IpcError) -> Self {
        Self::Ipc(error)
    }
}

#[cfg(unix)]
impl From<ProcessError> for DaemonServiceError {
    fn from(error: ProcessError) -> Self {
        Self::Process(error)
    }
}

#[cfg(test)]
mod tests {
    use super::DaemonRuntime;
    #[cfg(unix)]
    use super::{DaemonService, DaemonServiceError, IpcError, IpcServer};
    use carapana_protocol::{
        Autonomy, DaemonErrorCode, DaemonRequest, DaemonResponse, Envelope,
        MAX_WORKSPACE_PATH_BYTES, MessageOrigin, QueuedMessage, Selection, SessionStatus, WorkMode,
        WorkspaceFileReviewStatus,
    };
    use carapana_storage::{SessionRegistry, StoredSessionStatus, WorkspaceMetadata};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    struct TestDatabase(PathBuf);

    #[cfg(unix)]
    struct TestDirectory(PathBuf);

    #[cfg(unix)]
    impl TestDirectory {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;

            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let temporary_root = if cfg!(target_os = "macos") {
                PathBuf::from("/tmp")
            } else {
                std::env::temp_dir()
            };
            let path = std::fs::canonicalize(temporary_root).unwrap().join(format!(
                "carapana-daemon-service-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    impl TestDatabase {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let temporary_root = if cfg!(target_os = "macos") {
                PathBuf::from("/tmp")
            } else {
                std::env::temp_dir()
            };
            let path = std::fs::canonicalize(temporary_root).unwrap().join(format!(
                "carapana-daemon-{}-{nonce}-{id}.sqlite3",
                std::process::id()
            ));
            Self(path)
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

    #[cfg(unix)]
    #[test]
    fn should_create_paused_sessions_and_persist_messages_without_starting_them() {
        let directory = TestDirectory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut runtime = DaemonRuntime::open(&database_path, 10).unwrap();
        let workspace_path = workspace.to_str().unwrap().to_owned();

        let created = runtime.handle(Envelope::new(DaemonRequest::CreateSession {
            workspace_path: workspace_path.clone(),
        }));
        let session_id = match &created.payload {
            DaemonResponse::SessionCreated { session_id } => session_id.clone(),
            _ => panic!("session creation did not return a session id"),
        };
        let created_json = serde_json::to_string(&created).unwrap();
        assert!(!created_json.contains(&workspace_path));

        let prompt = "private queued prompt";
        let queued = runtime.handle(Envelope::new(DaemonRequest::EnqueueMessage {
            session_id: session_id.clone(),
            text: prompt.into(),
            selection: Selection {
                profile: "operator-choice".into(),
                work: WorkMode::Plan,
                autonomy: Autonomy::Ask,
                provider: "operator-choice".into(),
                model: "operator-choice".into(),
                variant: "default".into(),
            },
            origin: MessageOrigin::Cli,
        }));
        assert!(matches!(
            queued.payload,
            DaemonResponse::MessageQueued {
                session_id: ref queued_session,
                ..
            }
                if queued_session == &session_id
        ));
        let queued_json = serde_json::to_string(&queued).unwrap();
        assert!(!queued_json.contains(prompt));
        assert!(!queued_json.contains(&workspace_path));

        let sessions = runtime.sessions().unwrap();
        let session = sessions
            .into_iter()
            .find(|session| session.session_id == session_id)
            .expect("created session should be persisted");
        assert_eq!(session.status, StoredSessionStatus::Paused);
        assert!(session.active_message.is_none());
        assert_eq!(session.queued_messages.len(), 1);
        assert_eq!(session.queued_messages[0].text, prompt);
        assert_eq!(session.event_sequence, 2);
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_invalid_workspace_and_message_input_without_echoing_it() {
        let directory = TestDirectory::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut runtime = DaemonRuntime::open(&database_path, 10).unwrap();
        let invalid_workspace = directory.0.join("missing-secret-workspace");
        let rejected_workspace = runtime.handle(Envelope::new(DaemonRequest::CreateSession {
            workspace_path: invalid_workspace.to_string_lossy().into_owned(),
        }));
        assert_eq!(
            rejected_workspace.payload,
            DaemonResponse::Error {
                code: DaemonErrorCode::WorkspaceCreationRejected,
            }
        );
        assert!(
            !serde_json::to_string(&rejected_workspace)
                .unwrap()
                .contains("missing-secret-workspace")
        );

        let oversized_workspace = "x".repeat(MAX_WORKSPACE_PATH_BYTES + 1);
        let rejected_oversized_workspace =
            runtime.handle(Envelope::new(DaemonRequest::CreateSession {
                workspace_path: oversized_workspace.clone(),
            }));
        assert_eq!(
            rejected_oversized_workspace.payload,
            DaemonResponse::Error {
                code: DaemonErrorCode::WorkspaceCreationRejected,
            }
        );
        assert!(
            !serde_json::to_string(&rejected_oversized_workspace)
                .unwrap()
                .contains(&oversized_workspace)
        );

        let rejected_message = runtime.handle(Envelope::new(DaemonRequest::EnqueueMessage {
            session_id: "not-a-real-session".into(),
            text: "   ".into(),
            selection: Selection {
                profile: "operator-choice".into(),
                work: WorkMode::Plan,
                autonomy: Autonomy::Ask,
                provider: "operator-choice".into(),
                model: "operator-choice".into(),
                variant: "default".into(),
            },
            origin: MessageOrigin::Cli,
        }));
        assert_eq!(
            rejected_message.payload,
            DaemonResponse::Error {
                code: DaemonErrorCode::InvalidQueuedMessage,
            }
        );
    }

    #[test]
    fn should_block_hibernation_for_active_work_and_wake_only_hibernated_sessions() {
        let path = TestDatabase::new();
        {
            let mut registry = SessionRegistry::open(&path.0).unwrap();
            registry.create("active-session", 10).unwrap();
            registry
                .enqueue("active-session", message("active-message"), 11)
                .unwrap();
            registry.start_next("active-session", 12).unwrap();
            registry.pause("active-session", 13).unwrap();
            registry.create("paused-session", 20).unwrap();
        }

        let mut runtime = DaemonRuntime::open(&path.0, 21).unwrap();
        let blocked = runtime.handle(Envelope::new(DaemonRequest::HibernateSession {
            session_id: "active-session".into(),
        }));
        assert_eq!(
            blocked.payload,
            DaemonResponse::Error {
                code: DaemonErrorCode::HibernationBlocked,
            }
        );

        let not_hibernated = runtime.handle(Envelope::new(DaemonRequest::WakeSession {
            session_id: "paused-session".into(),
        }));
        assert_eq!(
            not_hibernated.payload,
            DaemonResponse::Error {
                code: DaemonErrorCode::SessionNotHibernated,
            }
        );

        let sessions = runtime.sessions().unwrap();
        let active = sessions
            .iter()
            .find(|session| session.session_id == "active-session")
            .unwrap();
        assert_eq!(active.status, StoredSessionStatus::Paused);
        assert_eq!(active.active_message, Some(message("active-message")));
        let paused = sessions
            .iter()
            .find(|session| session.session_id == "paused-session")
            .unwrap();
        assert_eq!(paused.status, StoredSessionStatus::Paused);
        assert!(paused.active_message.is_none());
    }

    #[test]
    fn should_recover_before_exposing_registry_sessions_and_never_resume_work() {
        let path = TestDatabase::new();
        {
            let mut registry = SessionRegistry::open(&path.0).unwrap();
            registry.create("session-1", 10).unwrap();
            registry
                .enqueue("session-1", message("active"), 11)
                .unwrap();
            registry
                .enqueue("session-1", message("queued"), 12)
                .unwrap();
            registry.start_next("session-1", 13).unwrap();
        }

        let mut runtime = DaemonRuntime::open(&path.0, 20).unwrap();
        assert_eq!(runtime.startup_recovered().len(), 1);
        let recovered = &runtime.startup_recovered()[0];
        assert_eq!(recovered.status, StoredSessionStatus::Paused);
        assert_eq!(recovered.active_message, Some(message("active")));
        assert_eq!(recovered.queued_messages, vec![message("queued")]);
        assert!(recovered.recovery_needs_revalidation);
        assert!(recovered.active_work_uncertain);

        let listed = runtime.sessions().unwrap();
        assert_eq!(listed, runtime.startup_recovered());

        let response = runtime.handle(Envelope::new(DaemonRequest::ListSessions {}));
        let DaemonResponse::Sessions { sessions } = response.payload else {
            panic!("session listing should return a session response");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Paused);
        assert_eq!(sessions[0].queued_count, 1);
        assert!(sessions[0].has_active_message);
        assert!(sessions[0].recovery_needs_revalidation);
        assert!(sessions[0].active_work_uncertain);
    }

    #[cfg(unix)]
    #[test]
    fn should_not_expose_workspace_file_metadata_in_ipc_event_pages() {
        let database = TestDatabase::new();
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("private-name.txt"), b"private contents").unwrap();
        {
            let mut registry = SessionRegistry::open(&database.0).unwrap();
            registry
                .create_with_workspace(
                    "session-1",
                    WorkspaceMetadata::capture(&workspace_path).unwrap(),
                    10,
                )
                .unwrap();
            registry
                .observe_workspace_file_with_hash("session-1", "private-name.txt", 11)
                .unwrap();
        }

        let mut runtime = DaemonRuntime::open(&database.0, 20).unwrap();
        let batch = runtime.events_after("session-1", 0).unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].sequence, 1);
        assert_eq!(batch.next_sequence, 2);
        let serialized = serde_json::to_string(&batch).unwrap();
        assert!(!serialized.contains("private-name.txt"));
        assert!(!serialized.contains("private contents"));
        assert!(!serialized.contains("content_sha256"));
    }

    #[cfg(unix)]
    #[test]
    fn should_return_only_explicit_workspace_path_and_review_status_over_ipc_contract() {
        let database = TestDatabase::new();
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("review.txt"), b"private contents").unwrap();
        let workspace = WorkspaceMetadata::capture(&workspace_path).unwrap();
        {
            let mut registry = SessionRegistry::open(&database.0).unwrap();
            registry
                .create_with_workspace("session-1", workspace, 10)
                .unwrap();
            registry
                .observe_workspace_file_with_hash("session-1", "review.txt", 11)
                .unwrap();
            registry.create("without-workspace", 12).unwrap();
        }
        fs::write(
            workspace_path.join("review.txt"),
            b"changed private contents",
        )
        .unwrap();

        let mut runtime = DaemonRuntime::open(&database.0, 20).unwrap();
        let response = runtime.handle(Envelope::new(DaemonRequest::ReviewWorkspace {
            session_id: "session-1".into(),
        }));
        let DaemonResponse::WorkspaceReview { review } = response.payload else {
            panic!("workspace review should return a report");
        };
        assert_eq!(review.files.len(), 1);
        assert_eq!(review.files[0].path, "review.txt");
        assert_eq!(review.files[0].status, WorkspaceFileReviewStatus::Changed);
        let serialized = serde_json::to_string(&review).unwrap();
        assert!(!serialized.contains("private contents"));
        assert!(!serialized.contains("content_sha256"));
        assert!(!serialized.contains("mtime"));
        assert!(!serialized.contains(workspace_path.to_string_lossy().as_ref()));

        assert_eq!(
            runtime
                .handle(Envelope::new(DaemonRequest::ReviewWorkspace {
                    session_id: "missing-session".into(),
                }))
                .payload,
            DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::SessionNotFound,
            }
        );
        assert_eq!(
            runtime
                .handle(Envelope::new(DaemonRequest::ReviewWorkspace {
                    session_id: "without-workspace".into(),
                }))
                .payload,
            DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::WorkspaceReviewUnavailable,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_require_path_acknowledgment_and_explicit_resume_confirmation() {
        let database = TestDatabase::new();
        let directory = TestDirectory::new();
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(
            workspace_path.join("source.txt"),
            b"initial selected contents",
        )
        .unwrap();
        {
            let mut registry = SessionRegistry::open(&database.0).unwrap();
            registry
                .create_with_workspace(
                    "recovery-resume",
                    WorkspaceMetadata::capture(&workspace_path).unwrap(),
                    10,
                )
                .unwrap();
            registry
                .observe_workspace_file_with_hash("recovery-resume", "source.txt", 11)
                .unwrap();
            registry
                .enqueue("recovery-resume", message("active"), 12)
                .unwrap();
            registry
                .enqueue("recovery-resume", message("queued"), 13)
                .unwrap();
            registry.start_next("recovery-resume", 14).unwrap();
        }
        fs::write(
            workspace_path.join("source.txt"),
            b"operator reviewed changed contents",
        )
        .unwrap();
        let mut runtime = DaemonRuntime::open(&database.0, 20).unwrap();

        assert_eq!(
            runtime
                .handle(Envelope::new(DaemonRequest::ResumeRecovered {
                    session_id: "recovery-resume".into(),
                }))
                .payload,
            DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::RecoveryAcknowledgmentRequired,
            }
        );

        let acknowledgment =
            runtime.handle(Envelope::new(DaemonRequest::AcknowledgeRecoveryPath {
                session_id: "recovery-resume".into(),
                path: "source.txt".into(),
            }));
        assert_eq!(
            acknowledgment.payload,
            DaemonResponse::RecoveryPathAcknowledged {
                path: "source.txt".into(),
                status: WorkspaceFileReviewStatus::Changed,
            }
        );

        assert_eq!(
            runtime
                .handle(Envelope::new(DaemonRequest::ResumeRecovered {
                    session_id: "recovery-resume".into(),
                }))
                .payload,
            DaemonResponse::RecoveryResumed {
                session_id: "recovery-resume".into(),
            }
        );
        let session = runtime
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.session_id == "recovery-resume")
            .unwrap();
        assert_eq!(session.status, StoredSessionStatus::Active);
        assert!(!session.recovery_needs_revalidation);
        assert!(!session.active_work_uncertain);
        assert_eq!(session.active_message, Some(message("active")));
        assert_eq!(session.queued_messages, vec![message("queued")]);
    }

    #[test]
    fn should_derive_attention_from_recovery_state_in_a_stable_order() {
        let path = TestDatabase::new();
        {
            let mut registry = SessionRegistry::open(&path.0).unwrap();
            registry.create("z-normal", 1).unwrap();
            registry.create("b-recovered", 2).unwrap();
            registry
                .enqueue("b-recovered", message("uncertain"), 3)
                .unwrap();
            registry.start_next("b-recovered", 4).unwrap();
            registry.create("a-recovered", 5).unwrap();
            registry
                .enqueue("a-recovered", message("queued"), 6)
                .unwrap();
            registry.start_next("a-recovered", 7).unwrap();
        }

        let mut runtime = DaemonRuntime::open(&path.0, 10).unwrap();
        let response = runtime.handle(Envelope::new(DaemonRequest::ListAttention {}));
        let DaemonResponse::Attention { items } = response.payload else {
            panic!("attention request should return a derived queue");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].session_id, "a-recovered");
        assert_eq!(items[1].session_id, "b-recovered");
        assert!(items.iter().all(|item| {
            item.reason == carapana_protocol::AttentionReason::RecoveryReview
                && item.active_work_uncertain
                && item.event_sequence == 4
        }));

        let empty_response = runtime.handle(Envelope::new(DaemonRequest::ListSessions {}));
        assert!(matches!(
            empty_response.payload,
            DaemonResponse::Sessions { .. }
        ));
    }

    #[test]
    fn should_not_append_another_recovery_event_when_runtime_restarts_again() {
        let path = TestDatabase::new();
        {
            let mut registry = SessionRegistry::open(&path.0).unwrap();
            registry.create("session-1", 10).unwrap();
            registry
                .enqueue("session-1", message("active"), 11)
                .unwrap();
            registry.start_next("session-1", 12).unwrap();
        }

        let first = DaemonRuntime::open(&path.0, 20).unwrap();
        assert_eq!(first.startup_recovered().len(), 1);
        drop(first);
        let mut second = DaemonRuntime::open(&path.0, 30).unwrap();
        assert!(second.startup_recovered().is_empty());
        assert_eq!(second.sessions().unwrap()[0].event_sequence, 4);
    }

    #[cfg(unix)]
    #[test]
    fn should_recover_before_serving_from_a_composed_daemon_service() {
        use carapana_protocol::{DaemonRequest, DaemonResponse, Envelope};
        use std::{
            os::unix::fs::PermissionsExt,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            thread,
        };

        let directory = TestDirectory::new();
        let database_path = directory.0.join("sessions.sqlite3");
        {
            let mut registry = SessionRegistry::open(&database_path).unwrap();
            registry.create("session-1", 10).unwrap();
            registry
                .enqueue("session-1", message("active"), 11)
                .unwrap();
            registry
                .enqueue("session-1", message("queued"), 12)
                .unwrap();
            registry.start_next("session-1", 13).unwrap();
        }

        let socket_path = directory.0.join("daemon.sock");
        let mut service = DaemonService::open(&database_path, &socket_path, 20).unwrap();
        assert_eq!(service.startup_recovered().len(), 1);
        assert_eq!(
            service.startup_recovered()[0].status,
            StoredSessionStatus::Paused
        );
        assert!(service.startup_recovered()[0].active_work_uncertain);
        assert_eq!(
            fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            service.serve_until(server_shutdown.as_ref()).unwrap();
        });
        let response =
            super::ipc_request(&socket_path, Envelope::new(DaemonRequest::ListSessions {}))
                .unwrap();
        let DaemonResponse::Sessions { sessions } = response.payload else {
            panic!("the service should list sessions");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Paused);
        assert!(sessions[0].active_work_uncertain);

        let response =
            super::ipc_request(&socket_path, Envelope::new(DaemonRequest::ListAttention {}))
                .unwrap();
        let DaemonResponse::Attention { items } = response.payload else {
            panic!("the service should expose derived recovery attention");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].session_id, "session-1");
        assert!(items[0].active_work_uncertain);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn should_not_recover_sessions_when_another_service_owns_the_socket() {
        let directory = TestDirectory::new();
        let database_path = directory.0.join("sessions.sqlite3");
        {
            let mut registry = SessionRegistry::open(&database_path).unwrap();
            registry.create("session-1", 10).unwrap();
            registry
                .enqueue("session-1", message("active"), 11)
                .unwrap();
            registry.start_next("session-1", 12).unwrap();
        }

        let socket_path = directory.0.join("daemon.sock");
        let existing_server = IpcServer::bind(&socket_path).unwrap();
        assert!(matches!(
            DaemonService::open(&database_path, &socket_path, 20),
            Err(DaemonServiceError::Ipc(IpcError::UnsafeSocketPath(_)))
        ));

        let mut registry = SessionRegistry::open(&database_path).unwrap();
        let session = registry.list().unwrap().remove(0);
        assert_eq!(session.status, StoredSessionStatus::Active);
        assert!(!session.recovery_needs_revalidation);
        assert!(!session.active_work_uncertain);
        drop(existing_server);
    }
}
