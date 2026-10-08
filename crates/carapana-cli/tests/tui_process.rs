#![cfg(any(target_os = "linux", target_os = "macos"))]

use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
use carapana_storage::SessionRegistry;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
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
        let path = std::env::temp_dir().join(format!(
            "carapana-tui-process-{}-{nonce}-{id}",
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

#[test]
fn should_initialize_and_restore_a_real_read_only_tui_terminal() {
    let directory = PrivateDir::new();
    let data_home = directory.0.join("xdg");
    let carapana_data = data_home.join("carapana");
    fs::create_dir(&data_home).unwrap();
    fs::create_dir(&carapana_data).unwrap();
    fs::set_permissions(&carapana_data, fs::Permissions::from_mode(0o700)).unwrap();
    let database_path = carapana_data.join("sessions.sqlite3");
    let mut registry = SessionRegistry::open(&database_path).unwrap();
    registry.create("pty-session", 10).unwrap();
    registry
        .enqueue(
            "pty-session",
            QueuedMessage {
                id: "pty-queued-1".into(),
                text: "must remain queued".into(),
                origin: "pty-test".into(),
                selection: Selection {
                    profile: "ask".into(),
                    work: WorkMode::Plan,
                    autonomy: Autonomy::Ask,
                    provider: "mock".into(),
                    model: "mock-model".into(),
                    variant: "default".into(),
                },
            },
            11,
        )
        .unwrap();
    drop(registry);
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600)).unwrap();

    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/tui_pty_smoke.py"
        ))
        .arg(env!("CARGO_BIN_EXE_carapana"))
        .arg(&database_path)
        .output()
        .expect("Python 3 is required for the Unix PTY integration test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
