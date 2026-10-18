#![cfg(unix)]

use carapana_daemon::{
    DaemonService, ProcessError, ProcessShutdownPolicy, ProcessSpec, ProcessSupervisor,
};
use carapana_storage::SessionRegistry;
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

const PROCESS_HOST_ARGUMENT: &str = "__carapana_internal_process_host_v1";
static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct PrivateDir(PathBuf);

impl PrivateDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let path = fs::canonicalize("/tmp").unwrap().join(format!(
            "carapana-process-test-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn carapana_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_carapana"))
}

fn seed_session(database_path: &Path) {
    let mut registry = SessionRegistry::open(database_path).unwrap();
    registry.create("test-session", 20).unwrap();
}

fn sleep_binary() -> PathBuf {
    for path in ["/bin/sleep", "/usr/bin/sleep"] {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("test requires a local sleep executable");
}

fn spawn_sleep(
    supervisor: &mut ProcessSupervisor,
    policy: ProcessShutdownPolicy,
) -> carapana_daemon::OwnedProcessInfo {
    supervisor
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                sleep_binary(),
                vec!["30".into()],
                std::env::current_dir().unwrap(),
            )
            .with_shutdown_policy(policy),
        )
        .unwrap()
}

fn process_group(pid: u32) -> Pid {
    Pid::from_raw(i32::try_from(pid).unwrap())
}

fn assert_group_gone(pid: u32) {
    let group = process_group(pid);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match killpg(group, None) {
            Err(Errno::ESRCH) => return,
            Ok(()) | Err(Errno::EPERM) => {}
            Err(error) => panic!("process-group probe failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "owned process group remained alive"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn write_frame(writer: &mut impl Write, payload: &[u8]) {
    writer
        .write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
        .unwrap();
    writer.write_all(payload).unwrap();
    writer.flush().unwrap();
}

fn read_frame(reader: &mut impl Read) -> Vec<u8> {
    let mut length = [0; 4];
    reader.read_exact(&mut length).unwrap();
    let mut payload = vec![0; usize::try_from(u32::from_be_bytes(length)).unwrap()];
    reader.read_exact(&mut payload).unwrap();
    payload
}

fn wait_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "private process host did not exit"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn descendant_marker(directory: &Path) -> PathBuf {
    directory.join("descendant.pid")
}

fn wait_for_descendant_marker(marker: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(contents) = fs::read_to_string(marker)
            && let Ok(descendant_id) = contents.trim().parse::<u32>()
        {
            return descendant_id;
        }
        if Instant::now() >= deadline {
            let files = fs::read_dir(marker.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>();
            let contents = fs::read_to_string(marker).ok();
            panic!(
                "process-group fixture did not start: {}; files={files:?}; contents={contents:?}",
                marker.display(),
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn should_terminate_only_the_process_group_owned_by_the_supervisor() {
    let mut unrelated = ChildGuard(
        Command::new(sleep_binary())
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let process = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::Terminate);
    assert_ne!(process.process_id, process.process_group_id);
    let owned_processes = supervisor.owned_processes().unwrap();
    assert_eq!(owned_processes.len(), 1);
    assert_eq!(owned_processes[0], process);
    assert!(matches!(
        supervisor.stop_owned_process("other-session", &process.id),
        Err(ProcessError::ProcessNotFound)
    ));

    supervisor
        .stop_owned_process("test-session", &process.id)
        .unwrap();
    assert!(supervisor.owned_processes().unwrap().is_empty());
    assert_group_gone(process.process_group_id);
    assert!(unrelated.0.try_wait().unwrap().is_none());
}

#[test]
fn should_terminate_descendants_that_remain_in_the_owned_process_group() {
    let directory = PrivateDir::new();
    let current_executable = std::env::current_exe().unwrap();
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let process = supervisor
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                current_executable,
                vec![
                    "--ignored".into(),
                    "--exact".into(),
                    "process_group_descendant_fixture".into(),
                    "--nocapture".into(),
                ],
                &directory.0,
            ),
        )
        .unwrap();
    let marker = descendant_marker(&directory.0);
    let descendant_id = wait_for_descendant_marker(&marker);
    assert_ne!(descendant_id, process.process_id);

    supervisor
        .stop_owned_process("test-session", &process.id)
        .unwrap();
    assert_group_gone(process.process_group_id);
}

#[test]
fn should_clean_up_descendants_after_the_owned_leader_exits() {
    let directory = PrivateDir::new();
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let process = supervisor
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                std::env::current_exe().unwrap(),
                vec![
                    "--ignored".into(),
                    "--exact".into(),
                    "process_group_exit_fixture".into(),
                    "--nocapture".into(),
                ],
                &directory.0,
            ),
        )
        .unwrap();
    let marker = descendant_marker(&directory.0);
    let _descendant_id = wait_for_descendant_marker(&marker);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if supervisor.owned_processes().unwrap().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "exited process group was not reaped"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_group_gone(process.process_group_id);
}

#[test]
fn should_preserve_keep_running_descendants_after_the_owned_leader_exits() {
    let directory = PrivateDir::new();
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let process = supervisor
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                std::env::current_exe().unwrap(),
                vec![
                    "--ignored".into(),
                    "--exact".into(),
                    "process_group_exit_fixture".into(),
                    "--nocapture".into(),
                ],
                &directory.0,
            )
            .with_shutdown_policy(ProcessShutdownPolicy::KeepRunning),
        )
        .unwrap();
    let marker = descendant_marker(&directory.0);
    let _descendant_id = wait_for_descendant_marker(&marker);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if supervisor.owned_processes().unwrap().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "keep-running owner host did not exit"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let group = process_group(process.process_group_id);
    assert!(killpg(group, None).is_ok());
    killpg(group, Some(Signal::SIGKILL)).unwrap();
    assert_group_gone(process.process_group_id);
}

#[test]
fn should_preserve_keep_running_groups_on_shutdown_but_allow_explicit_cleanup() {
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let terminated = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::Terminate);
    let preserved = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::KeepRunning);
    supervisor.shutdown().unwrap();

    assert_group_gone(terminated.process_group_id);
    let group = process_group(preserved.process_group_id);
    assert!(killpg(group, None).is_ok());
    killpg(group, Some(Signal::SIGKILL)).unwrap();
    assert_group_gone(preserved.process_group_id);
}

#[test]
fn should_skip_keep_running_processes_when_stopping_one_session() {
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let terminated = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::Terminate);
    let preserved = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::KeepRunning);

    supervisor.stop_session_processes("test-session").unwrap();
    assert_group_gone(terminated.process_group_id);
    assert!(killpg(process_group(preserved.process_group_id), None).is_ok());
    supervisor
        .stop_owned_process("test-session", &preserved.id)
        .unwrap();
    assert_group_gone(preserved.process_group_id);
}

#[test]
fn should_restart_only_an_owned_process_under_a_new_identity() {
    let mut supervisor = ProcessSupervisor::new(carapana_binary()).unwrap();
    let previous = spawn_sleep(&mut supervisor, ProcessShutdownPolicy::Terminate);
    let restarted = supervisor
        .restart_owned_process(
            "test-session",
            &previous.id,
            ProcessSpec::new(
                sleep_binary(),
                vec!["30".into()],
                std::env::current_dir().unwrap(),
            ),
        )
        .unwrap();

    assert_ne!(restarted.id, previous.id);
    assert_eq!(restarted.session_id, previous.session_id);
    assert_group_gone(previous.process_group_id);
    assert_eq!(supervisor.owned_processes().unwrap(), [restarted]);
}

#[test]
fn should_terminate_process_group_when_daemon_control_pipe_closes() {
    let mut host = Command::new(carapana_binary())
        .arg(PROCESS_HOST_ARGUMENT)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut stdin = host.stdin.take().unwrap();
    let mut stdout = host.stdout.take().unwrap();
    let start = serde_json::json!({
        "executable": sleep_binary(),
        "arguments": ["30"],
        "working_directory": std::env::current_dir().unwrap(),
        "shutdown_policy": "terminate",
    });
    write_frame(&mut stdin, &serde_json::to_vec(&start).unwrap());
    let response: serde_json::Value = serde_json::from_slice(&read_frame(&mut stdout)).unwrap();
    assert_eq!(response["status"], "started");
    let process_group_id = u32::try_from(response["process_group_id"].as_u64().unwrap()).unwrap();

    drop(stdin);
    wait_child(&mut host);
    assert_group_gone(process_group_id);
}

#[test]
fn should_not_expose_the_internal_process_host_as_a_user_command() {
    let output = Command::new(carapana_binary())
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(!help.contains(PROCESS_HOST_ARGUMENT));
}

#[test]
fn should_stop_supervised_processes_when_the_daemon_service_shuts_down() {
    let directory = PrivateDir::new();
    let database_path = directory.0.join("sessions.sqlite3");
    seed_session(&database_path);
    let mut service = DaemonService::open_with_process_host(
        &database_path,
        directory.0.join("daemon.sock"),
        carapana_binary(),
        20,
    )
    .unwrap();
    let process = service
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                sleep_binary(),
                vec!["30".into()],
                std::env::current_dir().unwrap(),
            ),
        )
        .unwrap();

    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let server_thread = thread::spawn(move || service.serve_until(server_shutdown.as_ref()));
    shutdown.store(true, Ordering::Release);
    server_thread.join().unwrap().unwrap();
    assert_group_gone(process.process_group_id);
}

#[test]
fn should_reject_process_ownership_for_a_missing_session() {
    let directory = PrivateDir::new();
    let mut service = DaemonService::open_with_process_host(
        directory.0.join("sessions.sqlite3"),
        directory.0.join("daemon.sock"),
        carapana_binary(),
        20,
    )
    .unwrap();
    let error = service
        .spawn_owned_process(
            "not-a-session",
            ProcessSpec::new(
                sleep_binary(),
                vec!["30".into()],
                std::env::current_dir().unwrap(),
            ),
        )
        .unwrap_err();
    assert!(matches!(error, ProcessError::OwnerSessionMissing));
    assert!(service.owned_processes().unwrap().is_empty());
}

#[test]
fn should_reject_a_non_host_executable_without_leaking_specification() {
    let mut supervisor = ProcessSupervisor::new(sleep_binary()).unwrap();
    let error = supervisor
        .spawn_owned_process(
            "test-session",
            ProcessSpec::new(
                "/bin/sleep",
                vec!["30".into()],
                std::env::current_dir().unwrap(),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessError::HostProtocol | ProcessError::HostRejected | ProcessError::Io(_)
    ));
    assert!(!error.to_string().contains("/bin/sleep"));
}

#[test]
#[ignore = "spawned by the process-group ownership integration test"]
fn process_group_descendant_fixture() {
    let mut descendant = Command::new(sleep_binary())
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    fs::write(Path::new("descendant.pid"), descendant.id().to_string()).unwrap();
    loop {
        if let Some(status) = descendant.try_wait().unwrap() {
            panic!("descendant unexpectedly exited: {status}");
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[test]
#[ignore = "spawned by the process-group exit integration test"]
fn process_group_exit_fixture() {
    let mut shell = Command::new("/bin/sh")
        .arg("-c")
        .arg("\"$2\" 30 & echo $! > \"$1\"")
        .arg("carapana-process-fixture")
        .arg("descendant.pid")
        .arg(sleep_binary())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = shell.wait().unwrap();
    assert!(status.success());
}
