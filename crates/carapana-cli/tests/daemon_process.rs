#![cfg(any(target_os = "linux", target_os = "macos"))]

use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
use carapana_storage::{SessionRegistry, StoredSessionStatus, WorkspaceMetadata};
use std::{
    fs,
    io::Read,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
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
    fs::create_dir_all(&home).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_carapana"))
        .arg("daemon")
        .env("HOME", home)
        .env("XDG_DATA_HOME", &data_home)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (child, user_data_directory(directory).join("daemon.sock"))
}

fn user_data_directory(directory: &PrivateDir) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        return directory.0.join("xdg/carapana");
    }
    #[cfg(target_os = "macos")]
    {
        return directory
            .0
            .join("home/Library/Application Support/Carapana");
    }
    #[allow(unreachable_code)]
    directory.0.join("xdg/carapana")
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
    let data_directory = user_data_directory(directory);
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
fn should_keep_recovered_work_paused_after_unchanged_cli_workspace_review() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join("workspace");
    fs::create_dir(&workspace_path).unwrap();
    fs::write(workspace_path.join("source.txt"), b"before crash").unwrap();
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "recovery-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        registry
            .observe_workspace_file("recovery-review", "source.txt", 11)
            .unwrap();
        registry
            .enqueue(
                "recovery-review",
                message("active-work", "must remain paused"),
                12,
            )
            .unwrap();
        registry
            .enqueue(
                "recovery-review",
                message("queued-work", "must remain queued"),
                13,
            )
            .unwrap();
        registry.start_next("recovery-review", 14).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let registry = SessionRegistry::open(&database_path).unwrap();
    let before_review = registry.get("recovery-review").unwrap();
    assert_eq!(before_review.status, StoredSessionStatus::Paused);
    assert!(before_review.recovery_needs_revalidation);
    assert!(before_review.active_work_uncertain);

    let review_output = run_cli(
        &directory,
        &["review-workspace", "recovery-review", "--json"],
    );
    assert!(
        review_output.status.success(),
        "workspace review failed: {}",
        String::from_utf8_lossy(&review_output.stderr)
    );
    let review_stdout =
        String::from_utf8(review_output.stdout).expect("review JSON must be valid UTF-8");
    let review: serde_json::Value = serde_json::from_str(&review_stdout).unwrap();
    assert_eq!(review.as_array().unwrap().len(), 1);
    assert_eq!(review[0]["path"], "source.txt");
    assert_eq!(review[0]["status"], "unchanged");
    assert_eq!(review[0].as_object().unwrap().len(), 2);
    assert!(!review_stdout.contains("before crash"));

    let repeated_review_output = run_cli(
        &directory,
        &["review-workspace", "recovery-review", "--json"],
    );
    assert!(
        repeated_review_output.status.success(),
        "repeated workspace review failed: {}",
        String::from_utf8_lossy(&repeated_review_output.stderr)
    );
    let repeated_review: serde_json::Value =
        serde_json::from_slice(&repeated_review_output.stdout).unwrap();
    assert_eq!(repeated_review, review);

    let inspected = run_cli(&directory, &["show", "recovery-review", "--json"]);
    assert!(inspected.status.success());
    let inspected: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let snapshot = &inspected["snapshot"];
    assert_eq!(snapshot["status"], "paused");
    assert!(snapshot["recovery_needs_revalidation"].as_bool().unwrap());
    assert!(snapshot["active_work_uncertain"].as_bool().unwrap());
    assert_eq!(snapshot["active_message"]["id"], "active-work");
    assert_eq!(snapshot["queued_messages"][0]["id"], "queued-work");

    let after_review = registry.get("recovery-review").unwrap();
    assert_eq!(after_review.status, before_review.status);
    assert_eq!(
        after_review.recovery_needs_revalidation,
        before_review.recovery_needs_revalidation
    );
    assert_eq!(
        after_review.active_work_uncertain,
        before_review.active_work_uncertain
    );
    assert_eq!(after_review.active_message, before_review.active_message);
    assert_eq!(after_review.queued_messages, before_review.queued_messages);
    assert_eq!(after_review.event_sequence, before_review.event_sequence);
    let observation = after_review
        .workspace_files
        .iter()
        .find(|observation| observation.relative_path().as_path() == Path::new("source.txt"))
        .expect("the explicit stat-only workspace observation should remain persisted");
    assert_eq!(observation.content_sha256(), None);

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_fail_workspace_review_without_workspace_without_changing_recovery_state() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("no-workspace-review", 10).unwrap();
        registry
            .enqueue(
                "no-workspace-review",
                message("active-work", "must remain paused"),
                11,
            )
            .unwrap();
        registry
            .enqueue(
                "no-workspace-review",
                message("queued-work", "must remain queued"),
                12,
            )
            .unwrap();
        registry.start_next("no-workspace-review", 13).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let mut registry = SessionRegistry::open(&database_path).unwrap();
    let before_review = registry.get("no-workspace-review").unwrap();
    assert_eq!(before_review.status, StoredSessionStatus::Paused);
    assert!(before_review.recovery_needs_revalidation);
    assert!(before_review.active_work_uncertain);
    assert!(before_review.workspace_files.is_empty());
    let (before_events, before_event_cursor, before_has_more) = registry
        .session_events_after("no-workspace-review", 0, 100)
        .unwrap();
    assert!(!before_has_more);
    assert_eq!(before_event_cursor, before_review.event_sequence);

    let review = run_cli(
        &directory,
        &["review-workspace", "no-workspace-review", "--json"],
    );
    assert!(!review.status.success());
    assert!(review.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&review.stderr);
    assert!(stderr.contains("WorkspaceReviewUnavailable"));
    assert!(!stderr.contains("WorkspaceNotConfigured"));
    assert!(!stderr.contains("workspace"));

    let inspected = run_cli(&directory, &["show", "no-workspace-review", "--json"]);
    assert!(inspected.status.success());
    let inspected: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let snapshot = &inspected["snapshot"];
    assert_eq!(snapshot["status"], "paused");
    assert!(snapshot["recovery_needs_revalidation"].as_bool().unwrap());
    assert!(snapshot["active_work_uncertain"].as_bool().unwrap());
    assert_eq!(snapshot["active_message"]["id"], "active-work");
    assert_eq!(snapshot["queued_messages"][0]["id"], "queued-work");

    let after_review = registry.get("no-workspace-review").unwrap();
    assert_eq!(after_review.status, before_review.status);
    assert_eq!(
        after_review.recovery_needs_revalidation,
        before_review.recovery_needs_revalidation
    );
    assert_eq!(
        after_review.active_work_uncertain,
        before_review.active_work_uncertain
    );
    assert_eq!(after_review.active_message, before_review.active_message);
    assert_eq!(after_review.queued_messages, before_review.queued_messages);
    assert_eq!(after_review.workspace_files, before_review.workspace_files);
    assert_eq!(after_review.event_sequence, before_review.event_sequence);
    let (after_events, after_event_cursor, after_has_more) = registry
        .session_events_after("no-workspace-review", 0, 100)
        .unwrap();
    assert!(!after_has_more);
    assert_eq!(after_events, before_events);
    assert_eq!(after_event_cursor, before_event_cursor);

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_fail_closed_without_leaking_workspace_path_when_root_identity_changes() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join(".SSH/workspace");
    fs::create_dir_all(&workspace_path).unwrap();
    fs::write(workspace_path.join("source.txt"), b"original").unwrap();
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "root-changed",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        registry
            .observe_workspace_file("root-changed", "source.txt", 11)
            .unwrap();
        registry
            .enqueue(
                "root-changed",
                message("active-work", "preserve active work"),
                12,
            )
            .unwrap();
        registry
            .enqueue(
                "root-changed",
                message("queued-work", "preserve queued work"),
                13,
            )
            .unwrap();
        registry.start_next("root-changed", 14).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let moved_workspace = workspace_path.with_extension("moved");
    fs::rename(&workspace_path, &moved_workspace).unwrap();
    symlink(&moved_workspace, &workspace_path).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let registry = SessionRegistry::open(&database_path).unwrap();
    let before_review = registry.get("root-changed").unwrap();
    assert_eq!(before_review.status, StoredSessionStatus::Paused);
    assert!(before_review.recovery_needs_revalidation);
    assert!(before_review.active_work_uncertain);

    let review = run_cli(&directory, &["review-workspace", "root-changed", "--json"]);
    assert!(!review.status.success());
    let stderr = String::from_utf8_lossy(&review.stderr);
    assert!(stderr.contains("WorkspaceReviewUnavailable"));
    assert!(!stderr.contains(workspace_path.to_string_lossy().as_ref()));
    assert!(!stderr.contains(moved_workspace.to_string_lossy().as_ref()));
    assert!(review.stdout.is_empty());

    let inspected = run_cli(&directory, &["show", "root-changed", "--json"]);
    assert!(inspected.status.success());
    let inspected: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let snapshot = &inspected["snapshot"];
    assert_eq!(snapshot["status"], "paused");
    assert!(snapshot["recovery_needs_revalidation"].as_bool().unwrap());
    assert!(snapshot["active_work_uncertain"].as_bool().unwrap());
    assert_eq!(snapshot["active_message"]["id"], "active-work");
    assert_eq!(snapshot["queued_messages"][0]["id"], "queued-work");

    let after_review = registry.get("root-changed").unwrap();
    assert_eq!(after_review.status, before_review.status);
    assert_eq!(
        after_review.recovery_needs_revalidation,
        before_review.recovery_needs_revalidation
    );
    assert_eq!(
        after_review.active_work_uncertain,
        before_review.active_work_uncertain
    );
    assert_eq!(after_review.active_message, before_review.active_message);
    assert_eq!(after_review.queued_messages, before_review.queued_messages);
    assert_eq!(after_review.event_sequence, before_review.event_sequence);

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_report_each_workspace_file_status_without_changing_recovery_state() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join("workspace");
    fs::create_dir(&workspace_path).unwrap();
    for (name, contents) in [
        ("unchanged.txt", b"unchanged".as_slice()),
        ("stat-changed.txt", b"stat baseline".as_slice()),
        ("changed.txt", b"original hash".as_slice()),
        ("missing.txt", b"will be removed".as_slice()),
        ("linked.txt", b"will become a symlink".as_slice()),
        ("large.txt", b"small baseline".as_slice()),
        ("target.txt", b"symlink target".as_slice()),
        (
            "unreadable.txt",
            b"synthetic unreadable file contents".as_slice(),
        ),
    ] {
        fs::write(workspace_path.join(name), contents).unwrap();
    }
    fs::create_dir(workspace_path.join("non-directory-parent")).unwrap();
    fs::write(
        workspace_path.join("non-directory-parent/file.txt"),
        b"directory will be replaced",
    )
    .unwrap();
    fs::write(
        workspace_path.join("max-hash.txt"),
        vec![b'm'; carapana_storage::MAX_WORKSPACE_FILE_HASH_BYTES],
    )
    .unwrap();
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "status-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        registry
            .observe_workspace_file("status-review", "unchanged.txt", 11)
            .unwrap();
        registry
            .observe_workspace_file("status-review", "stat-changed.txt", 12)
            .unwrap();
        registry
            .observe_workspace_file_with_hash("status-review", "changed.txt", 13)
            .unwrap();
        registry
            .observe_workspace_file("status-review", "missing.txt", 14)
            .unwrap();
        registry
            .observe_workspace_file("status-review", "linked.txt", 15)
            .unwrap();
        registry
            .observe_workspace_file_with_hash("status-review", "large.txt", 16)
            .unwrap();
        registry
            .observe_workspace_file_with_hash("status-review", "max-hash.txt", 17)
            .unwrap();
        registry
            .observe_workspace_file_with_hash("status-review", "unreadable.txt", 18)
            .unwrap();
        registry
            .observe_workspace_file("status-review", "non-directory-parent/file.txt", 19)
            .unwrap();
        registry
            .enqueue(
                "status-review",
                message("active-work", "preserve active work"),
                20,
            )
            .unwrap();
        registry
            .enqueue(
                "status-review",
                message("queued-work", "preserve queued work"),
                21,
            )
            .unwrap();
        registry.start_next("status-review", 22).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(
        workspace_path.join("stat-changed.txt"),
        b"stat metadata changed after capture",
    )
    .unwrap();
    fs::write(workspace_path.join("changed.txt"), b"operator changed file").unwrap();
    fs::remove_file(workspace_path.join("missing.txt")).unwrap();
    fs::remove_file(workspace_path.join("linked.txt")).unwrap();
    symlink(
        workspace_path.join("target.txt"),
        workspace_path.join("linked.txt"),
    )
    .unwrap();
    fs::write(
        workspace_path.join("large.txt"),
        vec![b'x'; carapana_storage::MAX_WORKSPACE_FILE_HASH_BYTES + 1],
    )
    .unwrap();
    fs::remove_dir_all(workspace_path.join("non-directory-parent")).unwrap();
    fs::write(
        workspace_path.join("non-directory-parent"),
        b"a regular file replacing the observed directory",
    )
    .unwrap();
    fs::set_permissions(
        workspace_path.join("unreadable.txt"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let mut registry = SessionRegistry::open(&database_path).unwrap();
    let before_review = registry.get("status-review").unwrap();
    let (before_events, before_event_cursor, before_has_more) = registry
        .session_events_after("status-review", 0, 100)
        .unwrap();
    assert!(!before_has_more);
    assert_eq!(before_event_cursor, before_review.event_sequence);
    assert_eq!(before_review.status, StoredSessionStatus::Paused);
    assert!(before_review.recovery_needs_revalidation);
    assert!(before_review.active_work_uncertain);

    let review_output = run_cli(&directory, &["review-workspace", "status-review", "--json"]);
    assert!(
        review_output.status.success(),
        "workspace review failed: {}",
        String::from_utf8_lossy(&review_output.stderr)
    );
    let review_stdout =
        String::from_utf8(review_output.stdout).expect("review JSON must be valid UTF-8");
    let review: serde_json::Value = serde_json::from_str(&review_stdout).unwrap();
    let files = review.as_array().unwrap();
    assert_eq!(files.len(), 9);
    for (index, (path, status)) in [
        ("unchanged.txt", "unchanged"),
        ("stat-changed.txt", "changed"),
        ("changed.txt", "changed"),
        ("missing.txt", "missing"),
        ("linked.txt", "unsafe"),
        ("large.txt", "too_large"),
        ("max-hash.txt", "unchanged"),
        ("unreadable.txt", "unreadable"),
        ("non-directory-parent/file.txt", "unsafe"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(files[index]["path"], path);
        assert_eq!(files[index]["status"], status);
        assert_eq!(files[index].as_object().unwrap().len(), 2);
    }
    assert!(!review_stdout.contains("operator changed file"));
    assert!(!review_stdout.contains("stat metadata changed after capture"));
    assert!(!review_stdout.contains("content_sha256"));
    assert!(!review_stdout.contains("mtime"));
    assert!(!review_stdout.contains("synthetic unreadable file contents"));
    assert!(!review_stdout.contains(workspace_path.to_string_lossy().as_ref()));

    assert_eq!(
        fs::read(workspace_path.join("unchanged.txt")).unwrap(),
        b"unchanged"
    );
    assert_eq!(
        fs::read(workspace_path.join("stat-changed.txt")).unwrap(),
        b"stat metadata changed after capture"
    );
    assert_eq!(
        fs::read(workspace_path.join("changed.txt")).unwrap(),
        b"operator changed file"
    );
    assert!(!workspace_path.join("missing.txt").exists());
    let linked_metadata = fs::symlink_metadata(workspace_path.join("linked.txt")).unwrap();
    assert!(linked_metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_link(workspace_path.join("linked.txt")).unwrap(),
        workspace_path.join("target.txt")
    );
    assert_eq!(
        fs::read(workspace_path.join("target.txt")).unwrap(),
        b"symlink target"
    );
    assert_eq!(
        fs::metadata(workspace_path.join("large.txt"))
            .unwrap()
            .len(),
        (carapana_storage::MAX_WORKSPACE_FILE_HASH_BYTES + 1) as u64
    );
    assert_eq!(
        fs::metadata(workspace_path.join("unreadable.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0
    );
    assert_eq!(
        fs::read(workspace_path.join("non-directory-parent")).unwrap(),
        b"a regular file replacing the observed directory"
    );

    let after_review = registry.get("status-review").unwrap();
    assert_eq!(after_review.status, before_review.status);
    assert_eq!(
        after_review.recovery_needs_revalidation,
        before_review.recovery_needs_revalidation
    );
    assert_eq!(
        after_review.active_work_uncertain,
        before_review.active_work_uncertain
    );
    assert_eq!(after_review.active_message, before_review.active_message);
    assert_eq!(after_review.queued_messages, before_review.queued_messages);
    assert_eq!(after_review.event_sequence, before_review.event_sequence);
    let (after_events, after_event_cursor, after_has_more) = registry
        .session_events_after("status-review", 0, 100)
        .unwrap();
    assert!(!after_has_more);
    assert_eq!(after_events, before_events);
    assert_eq!(after_event_cursor, before_event_cursor);
    let unreadable_digest = after_review
        .workspace_files
        .iter()
        .find(|observation| observation.relative_path().as_path() == Path::new("unreadable.txt"))
        .and_then(|observation| observation.content_sha256())
        .expect("the explicitly hashed unreadable fixture should retain its local digest");
    assert!(!review_stdout.contains(unreadable_digest));
    let stat_only_observation = after_review
        .workspace_files
        .iter()
        .find(|observation| observation.relative_path().as_path() == Path::new("stat-changed.txt"))
        .expect("the stat-only fixture should remain persisted");
    assert_eq!(stat_only_observation.content_sha256(), None);
    let expected_digest = after_review
        .workspace_files
        .iter()
        .find(|observation| observation.relative_path().as_path() == Path::new("max-hash.txt"))
        .and_then(|observation| observation.content_sha256())
        .expect("the explicitly hashed fixture should have a persisted digest");
    assert!(!review_stdout.contains(expected_digest));
    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_keep_sensitive_root_workspace_review_relative_and_read_only_over_daemon_cli() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join(".SSH/workspace");
    fs::create_dir_all(&workspace_path).unwrap();
    fs::write(
        workspace_path.join("selected.txt"),
        b"synthetic initial contents",
    )
    .unwrap();
    let external_target = directory.0.join("outside-target.txt");
    fs::write(&external_target, b"synthetic external target contents").unwrap();

    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "sensitive-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        registry
            .observe_workspace_file("sensitive-review", "selected.txt", 11)
            .unwrap();
        registry
            .enqueue(
                "sensitive-review",
                message("active-work", "preserve paused work"),
                12,
            )
            .unwrap();
        registry
            .enqueue(
                "sensitive-review",
                message("queued-work", "preserve queued work"),
                13,
            )
            .unwrap();
        registry.start_next("sensitive-review", 14).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(
        workspace_path.join("selected.txt"),
        b"changed synthetic private contents with a different size",
    )
    .unwrap();
    symlink(&external_target, workspace_path.join("unobserved-link.txt")).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let registry = SessionRegistry::open(&database_path).unwrap();
    let before_review = registry.get("sensitive-review").unwrap();
    assert_eq!(before_review.status, StoredSessionStatus::Paused);
    assert!(before_review.recovery_needs_revalidation);
    assert!(before_review.active_work_uncertain);
    assert_eq!(
        before_review.active_message,
        Some(message("active-work", "preserve paused work"))
    );
    assert_eq!(
        before_review.queued_messages,
        vec![message("queued-work", "preserve queued work")]
    );

    let review_output = run_cli(
        &directory,
        &["review-workspace", "sensitive-review", "--json"],
    );
    assert!(
        review_output.status.success(),
        "workspace review failed: {}",
        String::from_utf8_lossy(&review_output.stderr)
    );
    let review_stdout = String::from_utf8(review_output.stdout)
        .expect("sensitive workspace review JSON must be valid UTF-8");
    let review: serde_json::Value = serde_json::from_str(&review_stdout).unwrap();
    let files = review.as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "selected.txt");
    assert_eq!(files[0]["status"], "changed");
    assert_eq!(files[0].as_object().unwrap().len(), 2);
    for private_value in [
        workspace_path.to_string_lossy().into_owned(),
        external_target.to_string_lossy().into_owned(),
        "synthetic initial contents".to_owned(),
        "changed synthetic private contents".to_owned(),
        "synthetic external target contents".to_owned(),
        "content_sha256".to_owned(),
        "mtime".to_owned(),
    ] {
        assert!(!review_stdout.contains(&private_value));
    }

    let after_review = registry.get("sensitive-review").unwrap();
    assert_eq!(after_review.status, StoredSessionStatus::Paused);
    assert!(after_review.recovery_needs_revalidation);
    assert_eq!(
        after_review.active_work_uncertain,
        before_review.active_work_uncertain
    );
    assert_eq!(after_review.active_message, before_review.active_message);
    assert_eq!(after_review.queued_messages, before_review.queued_messages);
    assert_eq!(after_review.event_sequence, before_review.event_sequence);

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_report_a_missing_non_utf8_workspace_name_as_relative_valid_json() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join("workspace");
    fs::create_dir(&workspace_path).unwrap();

    let relative_path = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'n', 0xff, b'a', b'm', b'e', b'.', b't', b'x', b't',
    ]));
    fs::write(
        workspace_path.join(&relative_path),
        b"synthetic non-UTF-8 filename fixture contents",
    )
    .unwrap();
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "non-utf8-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        registry
            .observe_workspace_file("non-utf8-review", &relative_path, 11)
            .unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(workspace_path.join(&relative_path)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let output = run_cli(
        &directory,
        &["review-workspace", "non-utf8-review", "--json"],
    );
    assert!(
        output.status.success(),
        "workspace review failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("review JSON must be valid UTF-8");
    let review: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(review.as_array().unwrap().len(), 1);
    assert_eq!(review[0]["path"], relative_path.to_string_lossy().as_ref());
    assert_eq!(review[0]["status"], "missing");
    assert_eq!(review[0].as_object().unwrap().len(), 2);
    assert!(!stdout.contains(&workspace_path.to_string_lossy().into_owned()));
    assert!(!stdout.contains("synthetic non-UTF-8 filename fixture contents"));

    let registry = SessionRegistry::open(&database_path).unwrap();
    let session = registry.get("non-utf8-review").unwrap();
    assert_eq!(session.event_sequence, 2);
    assert_eq!(session.workspace_files.len(), 1);
    assert_eq!(session.status, StoredSessionStatus::Paused);

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_close_only_oversized_workspace_review_response_without_mutating_session() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join("workspace");
    let long_directory_name = "d".repeat(180);
    let files_directory = workspace_path.join(&long_directory_name);
    fs::create_dir_all(&files_directory).unwrap();
    let long_file_suffix = "x".repeat(220);
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "oversized-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                10,
            )
            .unwrap();
        for index in 0..200 {
            let file_name = format!("item-{index:04}-{long_file_suffix}");
            fs::write(files_directory.join(&file_name), b"selected file").unwrap();
            registry
                .observe_workspace_file(
                    "oversized-review",
                    format!("{long_directory_name}/{file_name}"),
                    11 + index,
                )
                .unwrap();
        }
        registry
            .enqueue(
                "oversized-review",
                message("active-work", "preserve active work"),
                211,
            )
            .unwrap();
        registry
            .enqueue(
                "oversized-review",
                message("queued-work", "preserve queued work"),
                212,
            )
            .unwrap();
        registry.start_next("oversized-review", 213).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let before_output = run_cli(&directory, &["show", "oversized-review", "--json"]);
    assert!(before_output.status.success());
    let before: serde_json::Value = serde_json::from_slice(&before_output.stdout).unwrap();
    let before_snapshot = before["snapshot"].clone();
    assert_eq!(before_snapshot["status"], "paused");
    assert!(
        before_snapshot["recovery_needs_revalidation"]
            .as_bool()
            .unwrap()
    );
    assert!(before_snapshot["active_work_uncertain"].as_bool().unwrap());

    let review = run_cli(
        &directory,
        &["review-workspace", "oversized-review", "--json"],
    );
    assert!(!review.status.success());
    assert!(review.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&review.stderr);
    assert!(!stderr.contains(workspace_path.to_string_lossy().as_ref()));
    assert!(!stderr.contains("item-0000"));
    assert!(
        daemon.try_wait().unwrap().is_none(),
        "daemon exited on oversized response"
    );

    let after_output = run_cli(&directory, &["show", "oversized-review", "--json"]);
    assert!(
        after_output.status.success(),
        "daemon did not serve a subsequent request: {}",
        String::from_utf8_lossy(&after_output.stderr)
    );
    let after: serde_json::Value = serde_json::from_slice(&after_output.stdout).unwrap();
    assert_eq!(after["snapshot"], before_snapshot);

    let registry = SessionRegistry::open(&database_path).unwrap();
    let session = registry.get("oversized-review").unwrap();
    assert_eq!(session.status, StoredSessionStatus::Paused);
    assert!(session.recovery_needs_revalidation);
    assert!(session.active_work_uncertain);
    assert_eq!(
        session.active_message,
        Some(message("active-work", "preserve active work"))
    );
    assert_eq!(
        session.queued_messages,
        vec![message("queued-work", "preserve queued work")]
    );
    assert_eq!(
        session.event_sequence,
        before_snapshot["event_sequence"].as_i64().unwrap()
    );

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_return_complete_workspace_review_near_the_ipc_frame_limit() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
    fs::create_dir_all(&data_directory).unwrap();
    fs::set_permissions(&data_directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = data_directory.join("sessions.sqlite3");
    let workspace_path = directory.0.join("workspace");
    let long_directory_name = "d".repeat(250);
    let files_directory = workspace_path.join(&long_directory_name);
    fs::create_dir_all(&files_directory).unwrap();
    let long_file_suffix = "x".repeat(245);
    {
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry
            .create_with_workspace(
                "near-limit-review",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                100,
            )
            .unwrap();
        for index in 0..110 {
            let file_name = format!("item-{index:04}-{long_file_suffix}");
            fs::write(files_directory.join(&file_name), b"selected file").unwrap();
            registry
                .observe_workspace_file(
                    "near-limit-review",
                    format!("{long_directory_name}/{file_name}"),
                    101 + index,
                )
                .unwrap();
        }
        registry
            .enqueue(
                "near-limit-review",
                message("active-work", "preserve active work"),
                211,
            )
            .unwrap();
        registry
            .enqueue(
                "near-limit-review",
                message("queued-work", "preserve queued work"),
                212,
            )
            .unwrap();
        registry.start_next("near-limit-review", 213).unwrap();
    }
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    wait_for_socket(&mut daemon, &socket);

    let before_output = run_cli(&directory, &["show", "near-limit-review", "--json"]);
    assert!(before_output.status.success());
    let before: serde_json::Value = serde_json::from_slice(&before_output.stdout).unwrap();
    let before_snapshot = before["snapshot"].clone();
    assert_eq!(before_snapshot["status"], "paused");
    assert!(
        before_snapshot["recovery_needs_revalidation"]
            .as_bool()
            .unwrap()
    );
    assert!(before_snapshot["active_work_uncertain"].as_bool().unwrap());

    let review = run_cli(
        &directory,
        &["review-workspace", "near-limit-review", "--json"],
    );
    assert!(
        review.status.success(),
        "near-limit review failed: {}",
        String::from_utf8_lossy(&review.stderr)
    );
    assert!(review.stdout.len() > 56 * 1024);
    let files: serde_json::Value = serde_json::from_slice(&review.stdout).unwrap();
    let files = files.as_array().unwrap();
    assert_eq!(files.len(), 110);
    let compact_report = serde_json::to_vec(files).unwrap();
    assert!(compact_report.len() > 56 * 1024);
    assert!(compact_report.len() < 64 * 1024 - 256);
    assert_eq!(
        files[0]["path"],
        format!("{long_directory_name}/item-0000-{long_file_suffix}")
    );
    assert_eq!(
        files[109]["path"],
        format!("{long_directory_name}/item-0109-{long_file_suffix}")
    );
    assert!(files.iter().all(|file| file["status"] == "unchanged"));
    assert!(
        files
            .iter()
            .all(|file| file.as_object().is_some_and(|object| object.len() == 2))
    );
    assert!(!String::from_utf8_lossy(&review.stdout).contains("selected file"));
    assert!(!String::from_utf8_lossy(&review.stdout).contains("content_sha256"));
    assert!(
        !String::from_utf8_lossy(&review.stdout)
            .contains(workspace_path.to_string_lossy().as_ref())
    );

    let after_output = run_cli(&directory, &["show", "near-limit-review", "--json"]);
    assert!(after_output.status.success());
    let after: serde_json::Value = serde_json::from_slice(&after_output.stdout).unwrap();
    assert_eq!(after["snapshot"], before_snapshot);

    let registry = SessionRegistry::open(&database_path).unwrap();
    let session = registry.get("near-limit-review").unwrap();
    assert_eq!(session.status, StoredSessionStatus::Paused);
    assert!(session.recovery_needs_revalidation);
    assert!(session.active_work_uncertain);
    assert_eq!(
        session.active_message,
        Some(message("active-work", "preserve active work"))
    );
    assert_eq!(
        session.queued_messages,
        vec![message("queued-work", "preserve queued work")]
    );
    assert_eq!(
        session.event_sequence,
        before_snapshot["event_sequence"].as_i64().unwrap()
    );

    let (status, stderr) = stop_with_signal(&mut daemon, "-TERM");
    assert!(status.success(), "daemon did not stop cleanly: {stderr}");
    assert!(!socket.exists(), "daemon socket remained after shutdown");
}

#[test]
fn should_refuse_public_database_directory_without_recovering_or_serving() {
    let directory = PrivateDir::new();
    seed_interrupted_and_paused_sessions(&directory);

    let data_directory = user_data_directory(&directory);
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
    let data_directory = user_data_directory(&directory);
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
fn should_refuse_a_symlinked_database_path_ancestor_without_mutating_its_target() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDir::new();
    let home = directory.0.join("home");
    fs::create_dir_all(&home).unwrap();
    let external = directory.0.join("external");
    fs::create_dir(&external).unwrap();
    fs::set_permissions(&external, fs::Permissions::from_mode(0o700)).unwrap();

    #[cfg(target_os = "linux")]
    symlink(&external, directory.0.join("xdg")).unwrap();
    #[cfg(target_os = "macos")]
    symlink(&external, home.join("Library")).unwrap();

    let (daemon, socket) = start_daemon(&directory);
    let mut daemon = DaemonGuard(daemon);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("daemon did not reject the symlinked database path ancestor");
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
        "daemon accepted a symlinked path ancestor"
    );
    assert!(stderr.contains("refusing to use a non-private or unexpected database path"));
    assert!(
        !socket.exists(),
        "daemon created a socket through the symlink"
    );
    assert_eq!(fs::read_dir(&external).unwrap().count(), 0);
    #[cfg(target_os = "linux")]
    assert!(
        fs::symlink_metadata(directory.0.join("xdg"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    #[cfg(target_os = "macos")]
    assert!(
        fs::symlink_metadata(home.join("Library"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn should_refuse_a_newer_database_schema_without_rewriting_or_serving() {
    let directory = PrivateDir::new();
    let data_directory = user_data_directory(&directory);
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
