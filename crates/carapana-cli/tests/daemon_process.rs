#![cfg(unix)]

use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
use carapana_storage::{SessionRegistry, StoredSessionStatus};
use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime},
};

struct DaemonGuard(std::process::Child);

impl std::ops::Deref for DaemonGuard {
    type Target = std::process::Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DaemonGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct PrivateDir(PathBuf);

impl PrivateDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "carapana-cli-process-{}-{nonce}-{id}",
            std::process::id()
        ));
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

fn start_daemon(directory: &PrivateDir) -> (std::process::Child, PathBuf) {
    let home = directory.0.join("home");
    let data_home = directory.0.join("xdg");
    let child = Command::new(env!("CARGO_BIN_EXE_carapana"))
        .arg("daemon")
        .env("HOME", home)
        .env("XDG_DATA_HOME", &data_home)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (child, data_home.join("carapana/daemon.sock"))
}

fn stop_with_signal(
    child: &mut std::process::Child,
    signal: &str,
) -> (std::process::ExitStatus, String) {
    let result = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(result.success(), "could not send {signal} to daemon");
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (status, stderr)
}

fn wait_for_socket(child: &mut std::process::Child, socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if socket.exists() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("daemon exited before binding socket ({status}): {}", stderr);
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon did not bind socket before timeout");
}

fn assert_cli_reads_and_stops_daemon(signal: &str) {
    let directory = PrivateDir::new();
    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let binary = env!("CARGO_BIN_EXE_carapana");
    for command in ["sessions", "attention"] {
        let output = Command::new(binary)
            .args([command, "--json"])
            .env("HOME", directory.0.join("home"))
            .env("XDG_DATA_HOME", directory.0.join("xdg"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "carapana {command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
    }

    let (status, stderr) = stop_with_signal(&mut daemon, signal);
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

fn message(id: &str, text: &str) -> QueuedMessage {
    QueuedMessage {
        id: id.into(),
        text: text.into(),
        origin: "daemon-recovery-test".into(),
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

fn seed_interrupted_and_paused_sessions(directory: &PrivateDir) {
    let data_directory = directory.0.join("xdg/carapana");
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let mut registry = SessionRegistry::open(&database_path).unwrap();
    registry.create("interrupted", 10).unwrap();
    registry
        .enqueue(
            "interrupted",
            message("active-message", "do not repeat"),
            11,
        )
        .unwrap();
    registry
        .enqueue("interrupted", message("queued-message", "keep queued"), 12)
        .unwrap();
    registry.start_next("interrupted", 13).unwrap();
    registry.create("already-paused", 20).unwrap();
    registry
        .enqueue(
            "already-paused",
            message("paused-queue", "still waiting"),
            21,
        )
        .unwrap();
    drop(registry);
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn run_cli(directory: &PrivateDir, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_carapana"))
        .args(arguments)
        .env("HOME", directory.0.join("home"))
        .env("XDG_DATA_HOME", directory.0.join("xdg"))
        .output()
        .unwrap()
}

#[test]
fn should_run_read_only_cli_against_daemon_and_shutdown_on_sigterm() {
    assert_cli_reads_and_stops_daemon("-TERM");
}

#[test]
fn should_run_read_only_cli_against_daemon_and_shutdown_on_sigint() {
    assert_cli_reads_and_stops_daemon("-INT");
}

#[test]
fn should_recover_only_interrupted_work_before_serving_real_cli_requests() {
    let directory = PrivateDir::new();
    seed_interrupted_and_paused_sessions(&directory);
    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let sessions = run_cli(&directory, &["sessions", "--json"]);
    assert!(
        sessions.status.success(),
        "sessions failed: {}",
        String::from_utf8_lossy(&sessions.stderr)
    );
    let sessions: serde_json::Value = serde_json::from_slice(&sessions.stdout).unwrap();
    let recovered = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == "interrupted")
        .unwrap();
    assert_eq!(recovered["status"], "paused");
    assert_eq!(recovered["queued_count"], 1);
    assert!(recovered["has_active_message"].as_bool().unwrap());
    assert!(recovered["recovery_needs_revalidation"].as_bool().unwrap());
    assert!(recovered["active_work_uncertain"].as_bool().unwrap());
    assert_eq!(recovered["attached_clients"], 0);

    let normal = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == "already-paused")
        .unwrap();
    assert_eq!(normal["status"], "paused");
    assert_eq!(normal["queued_count"], 1);
    assert!(!normal["recovery_needs_revalidation"].as_bool().unwrap());
    assert!(!normal["active_work_uncertain"].as_bool().unwrap());

    let attention = run_cli(&directory, &["attention", "--json"]);
    assert!(attention.status.success());
    let attention: serde_json::Value = serde_json::from_slice(&attention.stdout).unwrap();
    assert_eq!(attention.as_array().unwrap().len(), 1);
    assert_eq!(attention[0]["session_id"], "interrupted");
    assert!(attention[0]["active_work_uncertain"].as_bool().unwrap());

    let detail = run_cli(
        &directory,
        &["show", "interrupted", "--events-after", "0", "--json"],
    );
    assert!(detail.status.success());
    let detail: serde_json::Value = serde_json::from_slice(&detail.stdout).unwrap();
    let snapshot = &detail["snapshot"];
    assert_eq!(snapshot["status"], "paused");
    assert_eq!(snapshot["active_message"]["id"], "active-message");
    assert_eq!(snapshot["active_message"]["text"], "do not repeat");
    assert_eq!(snapshot["queued_messages"][0]["id"], "queued-message");
    assert_eq!(snapshot["queued_messages"][0]["text"], "keep queued");
    assert_eq!(snapshot["attached_clients"], 1);
    assert!(snapshot["recovery_needs_revalidation"].as_bool().unwrap());
    let events = detail["events"]["events"].as_array().unwrap();
    assert_eq!(events.last().unwrap()["event"]["type"], "recovered_paused");
    assert_eq!(
        events.last().unwrap()["event"]["active_message_id"],
        "active-message"
    );
    assert_eq!(detail["events"]["has_more"], false);

    let invalid_event_query = run_cli(
        &directory,
        &["show", "interrupted", "--events-after", "999", "--json"],
    );
    assert!(!invalid_event_query.status.success());
    assert!(
        String::from_utf8_lossy(&invalid_event_query.stderr).contains("InvalidEventCursor"),
        "CLI did not surface the invalid cursor error: {}",
        String::from_utf8_lossy(&invalid_event_query.stderr)
    );

    let sessions_after_show = run_cli(&directory, &["sessions", "--json"]);
    assert!(sessions_after_show.status.success());
    let sessions_after_show: serde_json::Value =
        serde_json::from_slice(&sessions_after_show.stdout).unwrap();
    let recovered_after_show = sessions_after_show
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == "interrupted")
        .unwrap();
    assert_eq!(recovered_after_show["attached_clients"], 0);

    let valid_event_query = run_cli(
        &directory,
        &["show", "interrupted", "--events-after", "0", "--json"],
    );
    assert!(valid_event_query.status.success());
    let valid_event_query: serde_json::Value =
        serde_json::from_slice(&valid_event_query.stdout).unwrap();
    assert_eq!(
        valid_event_query["snapshot"]["event_sequence"],
        snapshot["event_sequence"]
    );
    assert_eq!(
        valid_event_query["events"]["events"]
            .as_array()
            .unwrap()
            .len(),
        events.len()
    );

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_refuse_public_database_directory_without_recovering_or_serving() {
    let directory = PrivateDir::new();
    seed_interrupted_and_paused_sessions(&directory);

    let data_directory = directory.0.join("xdg/carapana");
    let database_path = data_directory.join("sessions.sqlite3");
    let registry = SessionRegistry::open(&database_path).unwrap();
    let before = registry.get("interrupted").unwrap();
    assert_eq!(before.status, StoredSessionStatus::Active);
    let before_sequence = before.event_sequence;
    drop(registry);

    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o755)).unwrap();
    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("daemon did not reject the public database directory");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();

    assert!(
        !status.success(),
        "daemon accepted a public database directory"
    );
    assert!(
        stderr.contains("non-private or unexpected database path"),
        "daemon did not report the unsafe path: {stderr}"
    );
    assert!(
        !socket.exists(),
        "daemon created a socket for unsafe storage"
    );
    assert_eq!(
        fs::metadata(&data_directory).unwrap().permissions().mode() & 0o777,
        0o755,
        "daemon should not silently change operator-owned permissions"
    );

    let registry = SessionRegistry::open(&database_path).unwrap();
    let after = registry.get("interrupted").unwrap();
    assert_eq!(after.status, StoredSessionStatus::Active);
    assert_eq!(after.active_message, before.active_message);
    assert_eq!(after.queued_messages, before.queued_messages);
    assert_eq!(after.event_sequence, before_sequence);
    assert!(!after.recovery_needs_revalidation);
    assert!(!after.active_work_uncertain);
}

#[test]
fn should_refuse_database_symlink_without_recovering_or_serving() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDir::new();
    let data_directory = directory.0.join("xdg/carapana");
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();

    let database_path = directory.0.join("external.sqlite3");
    let mut registry = SessionRegistry::open(&database_path).unwrap();
    registry.create("external-session", 10).unwrap();
    registry
        .enqueue(
            "external-session",
            message("active-message", "must not be recovered through a link"),
            11,
        )
        .unwrap();
    registry.start_next("external-session", 12).unwrap();
    let before = registry.get("external-session").unwrap();
    let before_sequence = before.event_sequence;
    drop(registry);
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let linked_database = data_directory.join("sessions.sqlite3");
    symlink(&database_path, &linked_database).unwrap();
    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("daemon did not reject the symlinked database path");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();

    assert!(
        !status.success(),
        "daemon followed a symlinked database path"
    );
    assert!(
        stderr.contains("non-private or unexpected database path"),
        "daemon did not report the unsafe path: {stderr}"
    );
    assert!(
        !socket.exists(),
        "daemon created a socket for a symlinked database"
    );
    assert!(
        fs::symlink_metadata(&linked_database)
            .unwrap()
            .file_type()
            .is_symlink(),
        "daemon replaced or removed the operator's symlink"
    );

    let registry = SessionRegistry::open(&database_path).unwrap();
    let after = registry.get("external-session").unwrap();
    assert_eq!(after.status, StoredSessionStatus::Active);
    assert_eq!(after.active_message, before.active_message);
    assert_eq!(after.queued_messages, before.queued_messages);
    assert_eq!(after.event_sequence, before_sequence);
    assert!(!after.recovery_needs_revalidation);
    assert!(!after.active_work_uncertain);
}

#[test]
fn should_refuse_a_newer_database_schema_without_rewriting_or_serving() {
    let directory = PrivateDir::new();
    let data_directory = directory.0.join("xdg/carapana");
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");

    let seed = Command::new("python3")
        .args([
            "-c",
            "import sqlite3, sys; connection = sqlite3.connect(sys.argv[1]); connection.execute('PRAGMA user_version = 99'); connection.close()",
        ])
        .arg(&database_path)
        .status()
        .unwrap();
    assert!(
        seed.success(),
        "could not create the future-schema SQLite fixture"
    );
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("daemon did not reject the newer database schema");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();

    assert!(!status.success(), "daemon accepted a newer database schema");
    assert!(
        stderr.contains("unsupported SQLite schema version: 99"),
        "daemon did not report the unsupported schema: {stderr}"
    );
    assert!(
        !socket.exists(),
        "daemon created a socket for an unsupported database schema"
    );
    assert_eq!(
        fs::metadata(&database_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let inspect = Command::new("python3")
        .args([
            "-c",
            "import sqlite3, sys; connection = sqlite3.connect(sys.argv[1]); print(connection.execute('PRAGMA user_version').fetchone()[0]); print(connection.execute('PRAGMA journal_mode').fetchone()[0]); connection.close()",
        ])
        .arg(&database_path)
        .output()
        .unwrap();
    assert!(inspect.status.success());
    assert_eq!(
        String::from_utf8_lossy(&inspect.stdout).trim(),
        "99\ndelete"
    );
}
