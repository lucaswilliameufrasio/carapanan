//! Safe startup orchestration for the future local daemon.
//!
//! This crate provides a local read-only IPC listener, but no provider or tool runner.

#[cfg(unix)]
mod ipc;

use std::{error::Error, fmt, path::Path};

use carapana_protocol::{
    DaemonErrorCode, DaemonRequest, DaemonResponse, Envelope, SessionStatus, SessionSummary,
};
use carapana_storage::{
    MigrationError, PersistedSession, SessionRegistry, SessionStoreError, UserDatabaseError,
};

#[cfg(unix)]
pub use ipc::{IpcError, IpcServer, request as ipc_request};

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

    /// Handle the initial read-only daemon contract. No message content is returned.
    pub fn handle(&mut self, request: Envelope<DaemonRequest>) -> Envelope<DaemonResponse> {
        let response = match request.payload {
            DaemonRequest::ListSessions {} => match self.sessions() {
                Ok(sessions) => DaemonResponse::Sessions {
                    sessions: sessions
                        .into_iter()
                        .map(|session| SessionSummary {
                            session_id: session.session_id,
                            status: match session.status {
                                carapana_storage::StoredSessionStatus::Active => {
                                    SessionStatus::Active
                                }
                                carapana_storage::StoredSessionStatus::Paused => {
                                    SessionStatus::Paused
                                }
                                carapana_storage::StoredSessionStatus::Hibernated => {
                                    SessionStatus::Hibernated
                                }
                            },
                            queued_count: u64::try_from(session.queued_messages.len())
                                .unwrap_or(u64::MAX),
                            has_active_message: session.active_message.is_some(),
                            recovery_needs_revalidation: session.recovery_needs_revalidation,
                            active_work_uncertain: session.active_work_uncertain,
                            updated_at_ms: session.updated_at_ms,
                        })
                        .collect(),
                },
                Err(_) => DaemonResponse::Error {
                    code: DaemonErrorCode::StorageUnavailable,
                },
            },
        };
        Envelope::new(response)
    }

    fn recover(mut registry: SessionRegistry, now_ms: i64) -> Result<Self, DaemonStartupError> {
        let startup_recovered = registry.recover_after_restart(now_ms)?;
        Ok(Self {
            registry,
            startup_recovered,
        })
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

#[cfg(test)]
mod tests {
    use super::DaemonRuntime;
    use carapana_protocol::{
        Autonomy, DaemonRequest, DaemonResponse, Envelope, QueuedMessage, Selection, SessionStatus,
        WorkMode,
    };
    use carapana_storage::{SessionRegistry, StoredSessionStatus};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    struct TestDatabase(PathBuf);

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
}
