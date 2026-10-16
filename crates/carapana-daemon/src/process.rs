//! Unix process ownership primitives for the local daemon.
//!
//! The process host is an internal mode of the `carapana` executable. It owns one
//! child process group and watches a private stdin pipe, so an unexpected daemon
//! exit closes the pipe and asks the host to terminate only that group.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const INTERNAL_HOST_ARGUMENT: &str = "__carapana_internal_process_host_v1";
const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_PROCESS_COUNT: usize = 32;
const MAX_ARGUMENT_COUNT: usize = 128;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const HOST_START_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const PROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(800);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(0);

/// Whether daemon/session shutdown should stop an owned process or preserve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessShutdownPolicy {
    Terminate,
    KeepRunning,
}

/// A shell-free process specification. Environment variables and stdio are not inherited.
pub struct ProcessSpec {
    executable: PathBuf,
    arguments: Vec<String>,
    working_directory: PathBuf,
    shutdown_policy: ProcessShutdownPolicy,
}

impl ProcessSpec {
    pub fn new(
        executable: impl Into<PathBuf>,
        arguments: Vec<String>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            arguments,
            working_directory: working_directory.into(),
            shutdown_policy: ProcessShutdownPolicy::Terminate,
        }
    }

    pub fn with_shutdown_policy(mut self, policy: ProcessShutdownPolicy) -> Self {
        self.shutdown_policy = policy;
        self
    }
}

/// Content-free identity for a process created by this supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedProcessInfo {
    pub id: String,
    pub session_id: String,
    pub process_id: u32,
    pub process_group_id: u32,
    pub shutdown_policy: ProcessShutdownPolicy,
}

#[derive(Debug)]
pub enum ProcessError {
    InvalidSessionId,
    OwnerSessionMissing,
    SessionStorageUnavailable,
    InvalidProcessSpec,
    ProcessLimitReached,
    ProcessNotFound,
    HostUnavailable,
    HostRejected,
    HostProtocol,
    HostTimedOut,
    ShutdownTimedOut,
    Io(io::Error),
    Serialization(serde_json::Error),
    Signal(Errno),
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSessionId => formatter.write_str("invalid process owner session"),
            Self::OwnerSessionMissing => formatter.write_str("process owner session not found"),
            Self::SessionStorageUnavailable => {
                formatter.write_str("process owner session could not be verified")
            }
            Self::InvalidProcessSpec => formatter.write_str("invalid owned-process specification"),
            Self::ProcessLimitReached => formatter.write_str("owned-process limit reached"),
            Self::ProcessNotFound => formatter.write_str("owned process not found"),
            Self::HostUnavailable => formatter.write_str("private process host is unavailable"),
            Self::HostRejected => formatter.write_str("private process host rejected the request"),
            Self::HostProtocol => formatter.write_str("private process host protocol failed"),
            Self::HostTimedOut => formatter.write_str("private process host startup timed out"),
            Self::ShutdownTimedOut => {
                formatter.write_str("owned processes did not stop before the deadline")
            }
            Self::Io(error) => write!(formatter, "owned-process I/O failed: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "owned-process protocol failed: {error}")
            }
            Self::Signal(error) => write!(formatter, "owned-process signal failed: {error}"),
        }
    }
}

impl Error for ProcessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::InvalidSessionId
            | Self::OwnerSessionMissing
            | Self::SessionStorageUnavailable
            | Self::InvalidProcessSpec
            | Self::ProcessLimitReached
            | Self::ProcessNotFound
            | Self::HostUnavailable
            | Self::HostRejected
            | Self::HostProtocol
            | Self::HostTimedOut
            | Self::ShutdownTimedOut
            | Self::Signal(_) => None,
        }
    }
}

impl From<io::Error> for ProcessError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProcessError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

struct ManagedProcess {
    info: OwnedProcessInfo,
    host: Child,
    control: ChildStdin,
}

/// Owns only process hosts that it spawned; it never searches by executable name or PID.
pub struct ProcessSupervisor {
    host_executable: PathBuf,
    processes: BTreeMap<String, ManagedProcess>,
    shutdown_started: bool,
}

impl ProcessSupervisor {
    pub fn for_current_executable() -> Result<Self, ProcessError> {
        let executable = std::env::current_exe().map_err(|_| ProcessError::HostUnavailable)?;
        Self::new(executable)
    }

    /// The supplied executable must implement the private process-host mode.
    pub fn new(host_executable: impl AsRef<Path>) -> Result<Self, ProcessError> {
        let executable = host_executable
            .as_ref()
            .canonicalize()
            .map_err(|_| ProcessError::HostUnavailable)?;
        validate_executable(&executable)?;
        Ok(Self {
            host_executable: executable,
            processes: BTreeMap::new(),
            shutdown_started: false,
        })
    }

    /// Start one explicitly specified executable without a shell or inherited environment.
    /// This API is not exposed through daemon IPC, CLI, or TUI requests.
    pub fn spawn_owned_process(
        &mut self,
        session_id: &str,
        spec: ProcessSpec,
    ) -> Result<OwnedProcessInfo, ProcessError> {
        if session_id.trim().is_empty()
            || session_id.len() > 256
            || session_id.chars().any(char::is_control)
        {
            return Err(ProcessError::InvalidSessionId);
        }
        if self.shutdown_started {
            return Err(ProcessError::HostUnavailable);
        }
        self.reap_finished()?;
        if self.processes.len() >= MAX_PROCESS_COUNT {
            return Err(ProcessError::ProcessLimitReached);
        }

        let (request, shutdown_policy) = validate_spec(spec)?;
        let mut host = Command::new(&self.host_executable);
        host.arg(INTERNAL_HOST_ARGUMENT)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = host.spawn().map_err(|_| ProcessError::HostUnavailable)?;
        let Some(mut control) = child.stdin.take() else {
            return Err(ProcessError::HostProtocol);
        };
        let Some(mut response_pipe) = child.stdout.take() else {
            drop(control);
            return Err(ProcessError::HostProtocol);
        };

        if let Err(error) = write_frame(&mut control, &request) {
            drop(control);
            return Err(error);
        }

        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let response = read_frame::<_, HostStartResponse>(&mut response_pipe);
            let _ = response_sender.send(response);
        });
        let response = match response_receiver.recv_timeout(HOST_START_TIMEOUT) {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                drop(control);
                wait_for_host_exit(&mut child, HOST_SHUTDOWN_TIMEOUT)?;
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drop(control);
                wait_for_host_exit(&mut child, HOST_SHUTDOWN_TIMEOUT)?;
                return Err(ProcessError::HostTimedOut);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                drop(control);
                wait_for_host_exit(&mut child, HOST_SHUTDOWN_TIMEOUT)?;
                return Err(ProcessError::HostProtocol);
            }
        };

        let (target_pid, process_group_id) = match response {
            HostStartResponse::Started {
                process_id,
                process_group_id,
            } if process_id > 0
                && i32::try_from(process_id).is_ok()
                && process_group_id == child.id() =>
            {
                (process_id, process_group_id)
            }
            HostStartResponse::Started { .. } => {
                let _ = write_frame(&mut control, &HostControl::Stop);
                drop(control);
                wait_for_host_exit(&mut child, HOST_SHUTDOWN_TIMEOUT)?;
                return Err(ProcessError::HostProtocol);
            }
            HostStartResponse::Rejected => {
                drop(control);
                wait_for_host_exit(&mut child, HOST_SHUTDOWN_TIMEOUT)?;
                return Err(ProcessError::HostRejected);
            }
        };

        let sequence = NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed);
        let id = format!("owned-{}-{sequence:x}", std::process::id());
        let info = OwnedProcessInfo {
            id: id.clone(),
            session_id: session_id.to_owned(),
            process_id: target_pid,
            process_group_id,
            shutdown_policy,
        };
        self.processes.insert(
            id,
            ManagedProcess {
                info: info.clone(),
                host: child,
                control,
            },
        );
        Ok(info)
    }

    /// List content-free identities for currently tracked process hosts.
    pub fn owned_processes(&mut self) -> Result<Vec<OwnedProcessInfo>, ProcessError> {
        self.reap_finished()?;
        Ok(self
            .processes
            .values()
            .map(|process| process.info.clone())
            .collect())
    }

    /// Explicitly stop one process group, including one marked `KeepRunning`.
    pub fn stop_owned_process(
        &mut self,
        session_id: &str,
        process_id: &str,
    ) -> Result<(), ProcessError> {
        let is_owned_by_session = self
            .processes
            .get(process_id)
            .is_some_and(|process| process.info.session_id == session_id);
        if !is_owned_by_session {
            return Err(ProcessError::ProcessNotFound);
        }
        let Some(mut process) = self.processes.remove(process_id) else {
            return Err(ProcessError::ProcessNotFound);
        };
        let result = write_frame(&mut process.control, &HostControl::Stop);
        drop(process.control);
        let wait_result = wait_for_host_exit(&mut process.host, HOST_SHUTDOWN_TIMEOUT);
        result?;
        wait_result
    }

    /// Replace one currently owned process with a newly specified process in the same session.
    pub fn restart_owned_process(
        &mut self,
        session_id: &str,
        process_id: &str,
        spec: ProcessSpec,
    ) -> Result<OwnedProcessInfo, ProcessError> {
        let is_owned_by_session = self
            .processes
            .get(process_id)
            .is_some_and(|process| process.info.session_id == session_id);
        if !is_owned_by_session {
            return Err(ProcessError::ProcessNotFound);
        }
        self.stop_owned_process(session_id, process_id)?;
        self.spawn_owned_process(session_id, spec)
    }

    /// Stop session-owned processes except those explicitly marked `KeepRunning`.
    pub fn stop_session_processes(&mut self, session_id: &str) -> Result<(), ProcessError> {
        let ids = self
            .processes
            .values()
            .filter(|process| {
                process.info.session_id == session_id
                    && process.info.shutdown_policy == ProcessShutdownPolicy::Terminate
            })
            .map(|process| process.info.id.clone())
            .collect::<Vec<_>>();
        let mut control_failed = false;
        for id in &ids {
            if let Some(process) = self.processes.get_mut(id)
                && write_frame(&mut process.control, &HostControl::Stop).is_err()
            {
                control_failed = true;
            }
        }

        let deadline = Instant::now() + HOST_SHUTDOWN_TIMEOUT;
        let mut exited = Vec::new();
        loop {
            exited.clear();
            for id in &ids {
                let Some(process) = self.processes.get_mut(id) else {
                    continue;
                };
                match process.host.try_wait() {
                    Ok(Some(_)) => exited.push(id.clone()),
                    Ok(None) => {}
                    Err(_) => control_failed = true,
                }
            }
            for id in &exited {
                self.processes.remove(id);
            }
            let pending = ids.iter().any(|id| self.processes.contains_key(id));
            if !pending {
                return if control_failed {
                    Err(ProcessError::HostProtocol)
                } else {
                    Ok(())
                };
            }
            if Instant::now() >= deadline {
                for id in &ids {
                    self.processes.remove(id);
                }
                return Err(ProcessError::ShutdownTimedOut);
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Stop all owned groups with one bounded deadline; `KeepRunning` groups are detached.
    pub fn shutdown(&mut self) -> Result<(), ProcessError> {
        if self.shutdown_started {
            return Ok(());
        }
        self.shutdown_started = true;

        let mut control_failed = false;
        for process in self.processes.values_mut() {
            let control = match process.info.shutdown_policy {
                ProcessShutdownPolicy::Terminate => HostControl::Stop,
                ProcessShutdownPolicy::KeepRunning => HostControl::Preserve,
            };
            if write_frame(&mut process.control, &control).is_err() {
                control_failed = true;
            }
        }

        let deadline = Instant::now() + HOST_SHUTDOWN_TIMEOUT;
        let mut exited = Vec::new();
        loop {
            exited.clear();
            for (id, process) in &mut self.processes {
                match process.host.try_wait() {
                    Ok(Some(_)) => exited.push(id.clone()),
                    Ok(None) => {}
                    Err(_) => control_failed = true,
                }
            }
            for id in &exited {
                self.processes.remove(id);
            }
            if self.processes.is_empty() {
                return if control_failed {
                    Err(ProcessError::HostProtocol)
                } else {
                    Ok(())
                };
            }
            if Instant::now() >= deadline {
                // Closing the pipe transfers cleanup to the still-running host; never kill
                // the host itself because that could orphan its process group.
                self.processes.clear();
                return Err(ProcessError::ShutdownTimedOut);
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn reap_finished(&mut self) -> Result<(), ProcessError> {
        let mut finished = Vec::new();
        for (id, process) in &mut self.processes {
            if process.host.try_wait()?.is_some() {
                finished.push(id.clone());
            }
        }
        for id in finished {
            self.processes.remove(&id);
        }
        Ok(())
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostStartRequest {
    executable: String,
    arguments: Vec<String>,
    working_directory: String,
    shutdown_policy: ProcessShutdownPolicy,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum HostStartResponse {
    Started {
        process_id: u32,
        process_group_id: u32,
    },
    Rejected,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HostControl {
    Stop,
    Preserve,
}

fn validate_spec(
    spec: ProcessSpec,
) -> Result<(HostStartRequest, ProcessShutdownPolicy), ProcessError> {
    let executable = spec
        .executable
        .canonicalize()
        .map_err(|_| ProcessError::InvalidProcessSpec)?;
    validate_executable(&executable).map_err(|_| ProcessError::InvalidProcessSpec)?;
    let working_directory = spec
        .working_directory
        .canonicalize()
        .map_err(|_| ProcessError::InvalidProcessSpec)?;
    if !working_directory.is_dir() {
        return Err(ProcessError::InvalidProcessSpec);
    }
    let executable = executable
        .to_str()
        .filter(|path| path.len() <= MAX_PATH_BYTES)
        .ok_or(ProcessError::InvalidProcessSpec)?
        .to_owned();
    let working_directory = working_directory
        .to_str()
        .filter(|path| path.len() <= MAX_PATH_BYTES)
        .ok_or(ProcessError::InvalidProcessSpec)?
        .to_owned();
    if spec.arguments.len() > MAX_ARGUMENT_COUNT
        || spec
            .arguments
            .iter()
            .any(|argument| argument.as_bytes().contains(&0))
        || spec.arguments.iter().map(String::len).sum::<usize>() > MAX_ARGUMENT_BYTES
    {
        return Err(ProcessError::InvalidProcessSpec);
    }
    Ok((
        HostStartRequest {
            executable,
            arguments: spec.arguments,
            working_directory,
            shutdown_policy: spec.shutdown_policy,
        },
        spec.shutdown_policy,
    ))
}

fn validate_executable(path: &Path) -> Result<(), ProcessError> {
    if !path.is_absolute() {
        return Err(ProcessError::InvalidProcessSpec);
    }
    let metadata = fs::metadata(path).map_err(|_| ProcessError::InvalidProcessSpec)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(ProcessError::InvalidProcessSpec);
    }
    Ok(())
}

/// Entry point for the private helper mode in the `carapana` executable.
/// It accepts one bounded launch frame, then one stop/preserve frame.
pub fn run_process_host() -> Result<(), ProcessError> {
    let terminate_on_signal = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(
        signal_hook::consts::SIGINT,
        Arc::clone(&terminate_on_signal),
    )
    .map_err(ProcessError::Io)?;
    signal_hook::flag::register(
        signal_hook::consts::SIGTERM,
        Arc::clone(&terminate_on_signal),
    )
    .map_err(ProcessError::Io)?;

    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let request = read_frame::<_, HostStartRequest>(&mut stdin_lock)?;
    drop(stdin_lock);
    let mut child = match spawn_process(&request) {
        Ok(child) => child,
        Err(_) => {
            let stdout = io::stdout();
            write_frame(&mut stdout.lock(), &HostStartResponse::Rejected)?;
            return Ok(());
        }
    };
    let process_id = child.id();
    let process_group_id = std::process::id();
    let stdout = io::stdout();
    if write_frame(
        &mut stdout.lock(),
        &HostStartResponse::Started {
            process_id,
            process_group_id,
        },
    )
    .is_err()
    {
        terminate_own_process_group()?;
        return Err(ProcessError::HostProtocol);
    }

    let (control_sender, control_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        let control = read_frame::<_, HostControl>(&mut stdin);
        // EOF or malformed control is fail-closed and terminates the owned group.
        let action = control.ok();
        let _ = control_sender.send(action);
    });

    loop {
        if terminate_on_signal.load(Ordering::Acquire) {
            terminate_own_process_group()?;
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                if request.shutdown_policy == ProcessShutdownPolicy::KeepRunning {
                    drop(child);
                    return Ok(());
                }
                terminate_own_process_group()?;
                return Ok(());
            }
            Ok(None) => {}
            Err(error) => {
                let cleanup_result = terminate_own_process_group();
                cleanup_result?;
                return Err(ProcessError::Io(error));
            }
        }
        match control_receiver.recv_timeout(POLL_INTERVAL) {
            Ok(Some(HostControl::Stop)) | Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate_own_process_group()?;
                return Ok(());
            }
            Ok(Some(HostControl::Preserve)) => {
                // Deliberate operator policy; this process is no longer owned for shutdown.
                drop(child);
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn spawn_process(request: &HostStartRequest) -> Result<Child, ProcessError> {
    if request.executable.len() > MAX_PATH_BYTES
        || request.working_directory.len() > MAX_PATH_BYTES
        || request.arguments.len() > MAX_ARGUMENT_COUNT
        || request.arguments.iter().map(String::len).sum::<usize>() > MAX_ARGUMENT_BYTES
    {
        return Err(ProcessError::InvalidProcessSpec);
    }
    let executable = PathBuf::from(&request.executable);
    let working_directory = PathBuf::from(&request.working_directory);
    validate_executable(&executable)?;
    if !working_directory.is_absolute() || !working_directory.is_dir() {
        return Err(ProcessError::InvalidProcessSpec);
    }
    let mut command = Command::new(executable);
    command
        .args(&request.arguments)
        .current_dir(working_directory)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(ProcessError::Io)
}

fn terminate_own_process_group() -> Result<(), ProcessError> {
    // The process host is the process-group leader and remains alive as its anchor
    // until SIGKILL. That prevents PGID reuse while cleanup is in progress.
    let process_group =
        Pid::from_raw(i32::try_from(std::process::id()).map_err(|_| ProcessError::HostProtocol)?);
    match killpg(process_group, Some(Signal::SIGTERM)) {
        Ok(()) => {}
        Err(error) => return Err(ProcessError::Signal(error)),
    }

    // The guardian itself is in the group, so group existence is not a completion
    // signal. Wait the fixed bounded grace period, then kill the entire group.
    thread::sleep(PROCESS_TERMINATION_GRACE);

    match killpg(process_group, Some(Signal::SIGKILL)) {
        Ok(()) => Ok(()),
        Err(error) => Err(ProcessError::Signal(error)),
    }
}

fn wait_for_host_exit(child: &mut Child, timeout: Duration) -> Result<(), ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ProcessError::HostTimedOut);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), ProcessError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(ProcessError::HostProtocol);
    }
    let size = u32::try_from(bytes.len()).map_err(|_| ProcessError::HostProtocol)?;
    writer.write_all(&size.to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, ProcessError> {
    let mut size = [0_u8; 4];
    reader.read_exact(&mut size)?;
    let size = usize::try_from(u32::from_be_bytes(size)).map_err(|_| ProcessError::HostProtocol)?;
    if size == 0 || size > MAX_FRAME_BYTES {
        return Err(ProcessError::HostProtocol);
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(ProcessError::from)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_FRAME_BYTES, ProcessError, ProcessShutdownPolicy, ProcessSpec, ProcessSupervisor,
        read_frame, validate_spec, write_frame,
    };
    use serde::{Deserialize, Serialize};
    use std::{io::Cursor, path::PathBuf};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct FrameFixture {
        value: String,
    }

    #[test]
    fn should_round_trip_bounded_internal_frames() {
        let frame = FrameFixture {
            value: "owned process".into(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        assert_eq!(
            read_frame::<_, FrameFixture>(&mut Cursor::new(bytes)).unwrap(),
            frame
        );
    }

    #[test]
    fn should_reject_oversized_internal_frames() {
        let mut bytes = u32::try_from(MAX_FRAME_BYTES + 1)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        bytes.resize(MAX_FRAME_BYTES + 5, 0);
        assert!(matches!(
            read_frame::<_, FrameFixture>(&mut Cursor::new(bytes)),
            Err(ProcessError::HostProtocol)
        ));
    }

    #[test]
    fn should_validate_shell_free_explicit_process_spec_and_shutdown_policy() {
        let executable = PathBuf::from("/bin/sleep");
        if !executable.exists() {
            return;
        }
        let directory = std::env::current_dir().unwrap();
        let (request, policy) = validate_spec(
            ProcessSpec::new(&executable, vec!["30".into()], &directory)
                .with_shutdown_policy(ProcessShutdownPolicy::KeepRunning),
        )
        .unwrap();
        assert_eq!(
            request.executable,
            executable.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(request.arguments, ["30"]);
        assert_eq!(policy, ProcessShutdownPolicy::KeepRunning);
        assert!(matches!(
            validate_spec(ProcessSpec::new("sleep", vec!["30".into()], &directory)),
            Err(ProcessError::InvalidProcessSpec)
        ));
    }

    #[test]
    fn should_reject_an_empty_or_control_character_session_owner() {
        let executable = PathBuf::from("/bin/sleep");
        if !executable.exists() {
            return;
        }
        let mut supervisor = ProcessSupervisor::for_current_executable().unwrap();
        let spec = ProcessSpec::new(
            &executable,
            vec!["30".into()],
            std::env::current_dir().unwrap(),
        );
        assert!(matches!(
            supervisor.spawn_owned_process("\n", spec),
            Err(ProcessError::InvalidSessionId)
        ));
        supervisor.shutdown().unwrap();
    }
}
