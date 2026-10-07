use std::{
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use carapana_protocol::{
    DaemonErrorCode, DaemonRequest, DaemonResponse, Envelope, SessionSnapshot,
};
use carapana_storage::{UserDatabaseError, user_database_path};

const MAX_FRAME_BYTES: usize = 64 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_NAME: &str = "daemon.sock";
const MAX_CLIENT_CONNECTIONS: usize = 128;

#[derive(Debug)]
pub enum IpcError {
    Io(io::Error),
    Serialization(serde_json::Error),
    InvalidFrame,
    UnsafeSocketPath(PathBuf),
    UserDatabase(UserDatabaseError),
    PersistentConnectionRequired,
    UnexpectedResponse,
    Daemon(DaemonErrorCode),
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "local daemon IPC failed: {error}"),
            Self::Serialization(error) => write!(formatter, "invalid local IPC message: {error}"),
            Self::InvalidFrame => formatter.write_str("invalid local IPC frame"),
            Self::UnsafeSocketPath(path) => write!(
                formatter,
                "refusing to use a socket outside a private directory or an existing path: {}",
                path.display()
            ),
            Self::UserDatabase(error) => write!(formatter, "could not resolve IPC path: {error}"),
            Self::PersistentConnectionRequired => {
                formatter.write_str("attach/detach requires a persistent IPC connection")
            }
            Self::UnexpectedResponse => {
                formatter.write_str("daemon returned a response incompatible with the request")
            }
            Self::Daemon(code) => write!(formatter, "daemon request failed: {code:?}"),
        }
    }
}

impl Error for IpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::UserDatabase(error) => Some(error),
            Self::InvalidFrame
            | Self::UnsafeSocketPath(_)
            | Self::PersistentConnectionRequired
            | Self::UnexpectedResponse
            | Self::Daemon(_) => None,
        }
    }
}

impl From<io::Error> for IpcError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<UserDatabaseError> for IpcError {
    fn from(error: UserDatabaseError) -> Self {
        Self::UserDatabase(error)
    }
}

/// Single-process local socket listener for the versioned session API.
///
/// Existing paths (including stale sockets) are never removed on startup. The caller
/// must resolve a stale endpoint explicitly rather than risk unlinking another process's
/// socket. A socket created here is unlinked on drop only if the path still identifies
/// the same filesystem entry.
pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl IpcServer {
    /// Bind the conventional socket beside this user's SQLite database.
    pub fn bind_user() -> Result<Self, IpcError> {
        let path = user_database_path()?.with_file_name(SOCKET_NAME);
        Self::bind(path)
    }

    /// Bind only inside an existing owner-private directory (no group/other access).
    pub fn bind(path: impl AsRef<Path>) -> Result<Self, IpcError> {
        let path = path.as_ref().to_owned();
        let parent = path
            .parent()
            .ok_or_else(|| IpcError::UnsafeSocketPath(path.clone()))?;
        let directory = fs::symlink_metadata(parent)
            .map_err(|_| IpcError::UnsafeSocketPath(parent.to_owned()))?;
        if !directory.file_type().is_dir() || directory.mode() & 0o077 != 0 {
            return Err(IpcError::UnsafeSocketPath(parent.to_owned()));
        }
        if fs::symlink_metadata(&path).is_ok() {
            return Err(IpcError::UnsafeSocketPath(path));
        }

        let listener = UnixListener::bind(&path)?;
        if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
            drop(listener);
            return Err(error.into());
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_socket() || metadata.mode() & 0o077 != 0 {
            drop(listener);
            return Err(IpcError::UnsafeSocketPath(path));
        }

        Ok(Self {
            listener,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Accept and handle exactly one one-shot request. Attachments require `serve_until`.
    pub fn accept_once(&self, runtime: &mut crate::DaemonRuntime) -> Result<(), IpcError> {
        let (mut stream, _) = self.listener.accept()?;
        handle_connection(&mut stream, runtime)
    }

    /// Serve clients until the caller sets `shutdown` to true.
    ///
    /// The loop multiplexes one-shot queries and persistent attachments. Malformed
    /// requests, disconnects, and failed response writes only remove that connection.
    /// EOF implicitly detaches the client; explicit detach sends an acknowledgement.
    /// The socket is polled nonblocking so shutdown remains responsive. The host is
    /// responsible for translating OS shutdown signals into the atomic flag.
    pub fn serve_until(
        &self,
        runtime: &mut crate::DaemonRuntime,
        shutdown: &AtomicBool,
    ) -> Result<(), IpcError> {
        self.listener.set_nonblocking(true)?;
        let mut clients = Vec::<ClientConnection>::new();
        let result = 'serve: loop {
            if shutdown.load(Ordering::Acquire) {
                break Ok(());
            }
            let mut did_work = false;
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        did_work = true;
                        if clients.len() >= MAX_CLIENT_CONNECTIONS {
                            drop(stream);
                            continue;
                        }
                        if stream.set_nonblocking(true).is_ok() {
                            clients.push(ClientConnection::new(stream));
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => break 'serve Err(IpcError::Io(error)),
                }
            }
            let mut index = 0;
            while index < clients.len() {
                let connected = clients[index].poll(runtime);
                did_work |= clients[index].did_work;
                clients[index].did_work = false;
                if connected {
                    index += 1;
                } else {
                    if let Some(session_id) = clients[index].attached_session.take() {
                        runtime.detach(&session_id);
                    }
                    clients.swap_remove(index);
                    did_work = true;
                }
            }
            if !did_work {
                thread::sleep(Duration::from_millis(10));
            }
        };
        for client in &mut clients {
            if let Some(session_id) = client.attached_session.take() {
                runtime.detach(&session_id);
            }
        }
        self.listener.set_nonblocking(false)?;
        result
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn handle_connection(
    stream: &mut UnixStream,
    runtime: &mut crate::DaemonRuntime,
) -> Result<(), IpcError> {
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    let request: Envelope<DaemonRequest> = read_json_frame(stream)?;
    let response: Envelope<DaemonResponse> = runtime.handle(request);
    write_json_frame(stream, &response)
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Send one versioned request and receive one response over the local socket.
pub fn request(
    path: impl AsRef<Path>,
    request: Envelope<DaemonRequest>,
) -> Result<Envelope<DaemonResponse>, IpcError> {
    if matches!(
        request.payload,
        DaemonRequest::Attach { .. } | DaemonRequest::EventsAfter { .. } | DaemonRequest::Detach {}
    ) {
        return Err(IpcError::PersistentConnectionRequired);
    }
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    write_json_frame(&mut stream, &request)?;
    read_json_frame(&mut stream)
}

/// Persistent attachment to a session. Dropping the handle detaches via EOF.
pub struct IpcAttachment {
    stream: UnixStream,
    snapshot: SessionSnapshot,
}

impl IpcAttachment {
    pub fn attach(path: impl AsRef<Path>, session_id: impl Into<String>) -> Result<Self, IpcError> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
        write_json_frame(
            &mut stream,
            &Envelope::new(DaemonRequest::Attach {
                session_id: session_id.into(),
            }),
        )?;
        let response: Envelope<DaemonResponse> = read_json_frame(&mut stream)?;
        let snapshot = match response.payload {
            DaemonResponse::Attached { snapshot } => snapshot,
            DaemonResponse::Error { code } => return Err(IpcError::Daemon(code)),
            _ => return Err(IpcError::UnexpectedResponse),
        };
        Ok(Self {
            stream,
            snapshot: *snapshot,
        })
    }

    pub fn snapshot(&self) -> &SessionSnapshot {
        &self.snapshot
    }

    /// Fetch the next bounded page after the last sequence the client applied.
    pub fn events_after(
        &mut self,
        after_sequence: i64,
    ) -> Result<carapana_protocol::SessionEventBatch, IpcError> {
        write_json_frame(
            &mut self.stream,
            &Envelope::new(DaemonRequest::EventsAfter { after_sequence }),
        )?;
        let response: Envelope<DaemonResponse> = read_json_frame(&mut self.stream)?;
        let batch = match response.payload {
            DaemonResponse::Events { batch } => batch,
            DaemonResponse::Error { code } => return Err(IpcError::Daemon(code)),
            _ => return Err(IpcError::UnexpectedResponse),
        };
        Ok(*batch)
    }

    pub fn detach(mut self) -> Result<u64, IpcError> {
        write_json_frame(&mut self.stream, &Envelope::new(DaemonRequest::Detach {}))?;
        let response: Envelope<DaemonResponse> = read_json_frame(&mut self.stream)?;
        let DaemonResponse::Detached {
            remaining_attached_clients,
        } = response.payload
        else {
            return Err(IpcError::UnexpectedResponse);
        };
        Ok(remaining_attached_clients)
    }
}

struct ClientConnection {
    stream: UnixStream,
    incoming: Vec<u8>,
    outgoing: Vec<u8>,
    attached_session: Option<String>,
    close_after_write: bool,
    did_work: bool,
}

impl ClientConnection {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            incoming: Vec::new(),
            outgoing: Vec::new(),
            attached_session: None,
            close_after_write: false,
            did_work: false,
        }
    }

    fn poll(&mut self, runtime: &mut crate::DaemonRuntime) -> bool {
        if !self.close_after_write {
            let mut buffer = [0; 8192];
            loop {
                match self.stream.read(&mut buffer) {
                    Ok(0) => return false,
                    Ok(read) => {
                        self.did_work = true;
                        self.incoming.extend_from_slice(&buffer[..read]);
                        if self.process_frames(runtime).is_err() {
                            return false;
                        }
                        if self.close_after_write {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return false,
                }
            }
        }

        while !self.outgoing.is_empty() {
            match self.stream.write(&self.outgoing) {
                Ok(0) => return false,
                Ok(written) => {
                    self.outgoing.drain(..written);
                    self.did_work = true;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        !(self.close_after_write && self.outgoing.is_empty())
    }

    fn process_frames(&mut self, runtime: &mut crate::DaemonRuntime) -> Result<(), IpcError> {
        loop {
            if self.incoming.len() < 4 {
                return Ok(());
            }
            let length = usize::try_from(u32::from_be_bytes(
                self.incoming[..4]
                    .try_into()
                    .map_err(|_| IpcError::InvalidFrame)?,
            ))
            .map_err(|_| IpcError::InvalidFrame)?;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(IpcError::InvalidFrame);
            }
            let frame_length = 4 + length;
            if self.incoming.len() < frame_length {
                return Ok(());
            }
            let frame = self.incoming[4..frame_length].to_vec();
            self.incoming.drain(..frame_length);
            let request: Envelope<DaemonRequest> = serde_json::from_slice(&frame)?;
            let response = self.handle_request(runtime, request);
            let mut encoded = Vec::new();
            write_json_frame(&mut encoded, &response)?;
            if self.outgoing.len() + encoded.len() > 2 * (MAX_FRAME_BYTES + 4) {
                return Err(IpcError::InvalidFrame);
            }
            self.outgoing.extend_from_slice(&encoded);
            self.did_work = true;
            if self.close_after_write {
                return Ok(());
            }
        }
    }

    fn handle_request(
        &mut self,
        runtime: &mut crate::DaemonRuntime,
        request: Envelope<DaemonRequest>,
    ) -> Envelope<DaemonResponse> {
        match (self.attached_session.as_deref(), request.payload) {
            (None, DaemonRequest::Attach { session_id }) => {
                let response = runtime.attach(&session_id);
                if matches!(response.payload, DaemonResponse::Attached { .. }) {
                    self.attached_session = Some(session_id);
                }
                response
            }
            (Some(session_id), DaemonRequest::Detach {}) => {
                let remaining_attached_clients = runtime.detach(session_id);
                self.attached_session = None;
                self.close_after_write = true;
                Envelope::new(DaemonResponse::Detached {
                    remaining_attached_clients,
                })
            }
            (None, DaemonRequest::Detach {}) => Envelope::new(DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::NotAttached,
            }),
            (Some(_), DaemonRequest::Attach { .. }) => Envelope::new(DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::AlreadyAttached,
            }),
            (None, DaemonRequest::EventsAfter { .. }) => Envelope::new(DaemonResponse::Error {
                code: carapana_protocol::DaemonErrorCode::NotAttached,
            }),
            (Some(session_id), DaemonRequest::EventsAfter { after_sequence }) => {
                match runtime.events_after(session_id, after_sequence) {
                    Ok(batch) => Envelope::new(DaemonResponse::Events {
                        batch: Box::new(batch),
                    }),
                    Err(error) => Envelope::new(DaemonResponse::Error {
                        code: match error {
                            carapana_storage::SessionStoreError::SessionNotFound => {
                                carapana_protocol::DaemonErrorCode::SessionNotFound
                            }
                            carapana_storage::SessionStoreError::InvalidEventCursor => {
                                carapana_protocol::DaemonErrorCode::InvalidEventCursor
                            }
                            _ => carapana_protocol::DaemonErrorCode::StorageUnavailable,
                        },
                    }),
                }
            }
            (_, request @ (DaemonRequest::ListSessions {} | DaemonRequest::ListAttention {})) => {
                runtime.handle(Envelope::new(request))
            }
        }
    }
}

fn read_json_frame<R, T>(reader: &mut R) -> Result<T, IpcError>
where
    R: Read,
    T: serde::de::DeserializeOwned,
{
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = usize::try_from(u32::from_be_bytes(header)).map_err(|_| IpcError::InvalidFrame)?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(IpcError::InvalidFrame);
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(serde_json::from_slice(&payload)?)
}

fn write_json_frame<W, T>(writer: &mut W, value: &T) -> Result<(), IpcError>
where
    W: Write,
    T: serde::Serialize,
{
    let payload = serde_json::to_vec(value)?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::InvalidFrame);
    }
    let length = u32::try_from(payload.len()).map_err(|_| IpcError::InvalidFrame)?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{IpcError, IpcServer, MAX_FRAME_BYTES, read_json_frame, write_json_frame};
    use crate::DaemonRuntime;
    use carapana_protocol::{
        Autonomy, DaemonRequest, DaemonResponse, Envelope, QueuedMessage, Selection, SessionStatus,
        WorkMode,
    };
    use carapana_storage::SessionRegistry;
    use std::{
        fs,
        io::{Cursor, Write},
        os::unix::fs::PermissionsExt,
        os::unix::net::UnixStream,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::{Duration, SystemTime},
    };

    struct PrivateDir(PathBuf);

    impl PrivateDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("carapana-ipc-{}-{nonce}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for PrivateDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn should_round_trip_session_listing_over_private_unix_socket() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let registry = SessionRegistry::open(&database_path).unwrap();
        drop(registry);
        let mut runtime = DaemonRuntime::open(&database_path, 10).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        assert_eq!(
            fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let server_thread = thread::spawn(move || server.accept_once(&mut runtime).unwrap());
        let response =
            super::request(&socket_path, Envelope::new(DaemonRequest::ListSessions {})).unwrap();
        server_thread.join().unwrap();
        assert_eq!(
            response,
            Envelope::new(DaemonResponse::Sessions { sessions: vec![] })
        );
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_refuse_public_directories_and_preserve_existing_paths() {
        let directory = PrivateDir::new();
        let broad = directory.0.join("broad");
        fs::create_dir(&broad).unwrap();
        fs::set_permissions(&broad, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            IpcServer::bind(broad.join("daemon.sock")),
            Err(IpcError::UnsafeSocketPath(_))
        ));

        let existing = directory.0.join("daemon.sock");
        fs::write(&existing, b"keep me").unwrap();
        assert!(matches!(
            IpcServer::bind(&existing),
            Err(IpcError::UnsafeSocketPath(_))
        ));
        assert_eq!(fs::read(existing).unwrap(), b"keep me");
    }

    #[test]
    fn should_reject_frames_larger_than_the_limit_before_allocating_payload() {
        let length = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
        assert!(matches!(
            read_json_frame::<_, serde_json::Value>(&mut Cursor::new(length)),
            Err(IpcError::InvalidFrame)
        ));

        let mut cursor = Cursor::new(Vec::new());
        assert!(matches!(
            write_json_frame(&mut cursor, &"x".repeat(MAX_FRAME_BYTES)),
            Err(IpcError::InvalidFrame)
        ));
    }

    #[test]
    fn should_continue_after_malformed_client_and_stop_on_shutdown() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let registry = SessionRegistry::open(&database_path).unwrap();
        drop(registry);
        let mut runtime = DaemonRuntime::open(&database_path, 10).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let mut malformed = UnixStream::connect(&socket_path).unwrap();
        let unsupported = br#"{"protocol":2,"payload":{"type":"list_sessions"}}"#;
        let length = u32::try_from(unsupported.len()).unwrap();
        malformed.write_all(&length.to_be_bytes()).unwrap();
        malformed.write_all(unsupported).unwrap();
        drop(malformed);

        let response =
            super::request(&socket_path, Envelope::new(DaemonRequest::ListSessions {})).unwrap();
        assert_eq!(
            response,
            Envelope::new(DaemonResponse::Sessions { sessions: vec![] })
        );

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    fn queued_message() -> QueuedMessage {
        QueuedMessage {
            id: "queued-1".into(),
            text: "preserve this work across attach".into(),
            origin: "tui".into(),
            selection: Selection {
                profile: "ask".into(),
                work: WorkMode::Plan,
                autonomy: Autonomy::Ask,
                provider: "mock".into(),
                model: "mock-model".into(),
                variant: "default".into(),
            },
        }
    }

    #[test]
    fn should_attach_multiple_clients_with_full_snapshot_and_detach_without_mutating_session() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        registry.enqueue("session-1", queued_message(), 11).unwrap();
        drop(registry);
        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let mut first = super::IpcAttachment::attach(&socket_path, "session-1").unwrap();
        assert_eq!(first.snapshot().status, SessionStatus::Paused);
        assert_eq!(first.snapshot().queued_messages, vec![queued_message()]);
        assert_eq!(first.snapshot().attached_clients, 1);

        let mut later_message = queued_message();
        later_message.id = "queued-2".into();
        later_message.text = "queued after the first snapshot".into();
        SessionRegistry::open(&database_path)
            .unwrap()
            .enqueue("session-1", later_message.clone(), 12)
            .unwrap();
        let catch_up = first.events_after(first.snapshot().event_sequence).unwrap();
        assert_eq!(catch_up.events.len(), 1);
        assert_eq!(catch_up.events[0].sequence, 3);
        assert_eq!(catch_up.next_sequence, 3);
        assert!(!catch_up.has_more);
        assert!(matches!(
            &catch_up.events[0].event,
            carapana_protocol::DaemonSessionEvent::MessageQueued { message }
                if message == &later_message
        ));
        assert!(matches!(
            first.events_after(999),
            Err(IpcError::Daemon(
                carapana_protocol::DaemonErrorCode::InvalidEventCursor
            ))
        ));

        let second = super::IpcAttachment::attach(&socket_path, "session-1").unwrap();
        assert_eq!(second.snapshot().attached_clients, 2);
        assert_eq!(
            second.snapshot().queued_messages,
            vec![queued_message(), later_message]
        );
        let listed =
            super::request(&socket_path, Envelope::new(DaemonRequest::ListSessions {})).unwrap();
        let DaemonResponse::Sessions { sessions } = listed.payload else {
            panic!("session listing should work while clients are attached");
        };
        assert_eq!(sessions[0].attached_clients, 2);

        assert_eq!(first.detach().unwrap(), 1);
        assert_eq!(second.detach().unwrap(), 0);
        let listed =
            super::request(&socket_path, Envelope::new(DaemonRequest::ListSessions {})).unwrap();
        let DaemonResponse::Sessions { sessions } = listed.payload else {
            panic!("session listing should work after detach");
        };
        assert_eq!(sessions[0].attached_clients, 0);
        assert_eq!(sessions[0].status, SessionStatus::Paused);
        assert_eq!(sessions[0].queued_count, 2);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_treat_connection_eof_as_detach_and_reconnect_with_a_fresh_snapshot() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        registry.enqueue("session-1", queued_message(), 11).unwrap();
        drop(registry);
        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let first = super::IpcAttachment::attach(&socket_path, "session-1").unwrap();
        drop(first);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let response =
                super::request(&socket_path, Envelope::new(DaemonRequest::ListSessions {}))
                    .unwrap();
            let DaemonResponse::Sessions { sessions } = response.payload else {
                panic!("session listing should succeed");
            };
            if sessions[0].attached_clients == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "EOF did not detach client"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let reconnected = super::IpcAttachment::attach(&socket_path, "session-1").unwrap();
        assert_eq!(reconnected.snapshot().attached_clients, 1);
        assert_eq!(
            reconnected.snapshot().queued_messages,
            vec![queued_message()]
        );
        assert_eq!(reconnected.detach().unwrap(), 0);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_require_persistent_connection_for_attach_and_detach_requests() {
        let socket_path = PathBuf::from("unused.sock");
        assert!(matches!(
            super::request(
                &socket_path,
                Envelope::new(DaemonRequest::Attach {
                    session_id: "session-1".into()
                })
            ),
            Err(IpcError::PersistentConnectionRequired)
        ));
        assert!(matches!(
            super::request(
                &socket_path,
                Envelope::new(DaemonRequest::EventsAfter { after_sequence: 0 })
            ),
            Err(IpcError::PersistentConnectionRequired)
        ));
    }

    #[test]
    fn should_clear_all_attachment_presence_on_controlled_service_shutdown() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        drop(registry);
        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
            runtime
        });

        let attachment = super::IpcAttachment::attach(&socket_path, "session-1").unwrap();
        assert_eq!(attachment.snapshot().attached_clients, 1);
        shutdown.store(true, Ordering::Release);
        let mut runtime = server_thread.join().unwrap();
        let listed = runtime.handle(Envelope::new(DaemonRequest::ListSessions {}));
        let DaemonResponse::Sessions { sessions } = listed.payload else {
            panic!("runtime should remain inspectable after controlled shutdown");
        };
        assert_eq!(sessions[0].attached_clients, 0);
        drop(attachment);
        assert!(!socket_path.exists());
    }
}
