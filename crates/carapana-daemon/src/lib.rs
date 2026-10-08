//! Safe startup orchestration for the future local daemon.
//!
//! This crate provides local session inspection/attachment IPC, but no provider or tool runner.

#[cfg(unix)]
mod ipc;

use std::{collections::HashMap, error::Error, fmt, path::Path};

#[cfg(unix)]
use std::sync::atomic::AtomicBool;

use carapana_protocol::{
    AttentionItem, AttentionReason, DaemonErrorCode, DaemonRequest, DaemonResponse,
    DaemonSessionEvent, Envelope, SessionEventBatch, SessionEventRecord, SessionSnapshot,
    SessionStatus, SessionSummary,
};
use carapana_storage::{
    MigrationError, PersistedSession, SessionRegistry, SessionStoreError, StoredSessionEvent,
    StoredSessionStatus, UserDatabaseError,
};

#[cfg(unix)]
pub use ipc::{IpcAttachment, IpcError, IpcServer, request as ipc_request};

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

    /// Handle a one-shot query. Attach/detach are handled by persistent IPC connections.
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
            DaemonRequest::ListAttention {} => match self.attention_queue() {
                Ok(items) => DaemonResponse::Attention { items },
                Err(_) => DaemonResponse::Error {
                    code: DaemonErrorCode::StorageUnavailable,
                },
            },
            DaemonRequest::Attach { .. } | DaemonRequest::Detach {} => DaemonResponse::Error {
                code: DaemonErrorCode::ConnectionRequired,
            },
            DaemonRequest::EventsAfter { .. } => DaemonResponse::Error {
                code: DaemonErrorCode::ConnectionRequired,
            },
        };
        Envelope::new(response)
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

/// Recovered daemon runtime bound to its local IPC listener.
///
/// Construction reserves the socket before paused crash recovery, preventing a second
/// daemon from recovering the same sessions. It accepts no requests until recovery has
/// completed and the host starts serving with its supplied shutdown signal.
#[cfg(unix)]
pub struct DaemonService {
    runtime: DaemonRuntime,
    server: IpcServer,
}

#[cfg(unix)]
#[derive(Debug)]
pub enum DaemonServiceError {
    Startup(DaemonStartupError),
    Ipc(IpcError),
}

#[cfg(unix)]
impl fmt::Display for DaemonServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup(error) => write!(formatter, "daemon service startup failed: {error}"),
            Self::Ipc(error) => write!(formatter, "daemon service IPC failed: {error}"),
        }
    }
}

#[cfg(unix)]
impl Error for DaemonServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Startup(error) => Some(error),
            Self::Ipc(error) => Some(error),
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
        Ok(Self { runtime, server })
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
        Ok(Self { runtime, server })
    }

    pub fn startup_recovered(&self) -> &[PersistedSession] {
        self.runtime.startup_recovered()
    }

    pub fn socket_path(&self) -> &Path {
        self.server.path()
    }

    pub fn serve_until(&mut self, shutdown: &AtomicBool) -> Result<(), IpcError> {
        self.server.serve_until(&mut self.runtime, shutdown)
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

#[cfg(test)]
mod tests {
    use super::DaemonRuntime;
    #[cfg(unix)]
    use super::{DaemonService, DaemonServiceError, IpcError, IpcServer};
    use carapana_protocol::{
        Autonomy, DaemonRequest, DaemonResponse, Envelope, QueuedMessage, Selection, SessionStatus,
        WorkMode,
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
            let path = std::env::temp_dir().join(format!(
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
            let path = std::env::temp_dir().join(format!(
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
                .observe_workspace_file("session-1", "private-name.txt", 11)
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
