use std::{
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use carapana_protocol::{DaemonRequest, DaemonResponse, Envelope};
use carapana_storage::{UserDatabaseError, user_database_path};

const MAX_FRAME_BYTES: usize = 64 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_NAME: &str = "daemon.sock";

#[derive(Debug)]
pub enum IpcError {
    Io(io::Error),
    Serialization(serde_json::Error),
    InvalidFrame,
    UnsafeSocketPath(PathBuf),
    UserDatabase(UserDatabaseError),
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
        }
    }
}

impl Error for IpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::UserDatabase(error) => Some(error),
            Self::InvalidFrame | Self::UnsafeSocketPath(_) => None,
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

/// Single-process local socket listener for the versioned, read-only daemon API.
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

    /// Accept and handle exactly one request. Call repeatedly from the daemon loop.
    pub fn accept_once(&self, runtime: &mut crate::DaemonRuntime) -> Result<(), IpcError> {
        let (mut stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
        let request = read_json_frame::<_, Envelope<DaemonRequest>>(&mut stream)?;
        let response: Envelope<DaemonResponse> = runtime.handle(request);
        write_json_frame(&mut stream, &response)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
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
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    write_json_frame(&mut stream, &request)?;
    read_json_frame(&mut stream)
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
    use carapana_protocol::{DaemonRequest, DaemonResponse, Envelope};
    use carapana_storage::SessionRegistry;
    use std::{
        fs,
        io::Cursor,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::SystemTime,
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
}
