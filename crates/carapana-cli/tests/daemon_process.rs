#![cfg(unix)]

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
    let (mut daemon, socket) = start_daemon(&directory);
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

#[test]
fn should_run_read_only_cli_against_daemon_and_shutdown_on_sigterm() {
    assert_cli_reads_and_stops_daemon("-TERM");
}

#[test]
fn should_run_read_only_cli_against_daemon_and_shutdown_on_sigint() {
    assert_cli_reads_and_stops_daemon("-INT");
}
