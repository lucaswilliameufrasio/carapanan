use std::process::Command;

#[cfg(unix)]
#[test]
fn should_open_pickers_and_resize_in_a_real_controlling_terminal() {
    let output = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pty_smoke.py"))
        .arg(env!("CARGO_BIN_EXE_carapana-prototype"))
        .output()
        .expect("Python 3 is required for the Unix PTY integration gate");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn should_return_structured_approval_and_exit_without_waiting() {
    let output = Command::new(env!("CARGO_BIN_EXE_carapana-prototype"))
        .args(["run", "--scenario", "approval", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["executed"], false);
    assert_eq!(result["prototype"], true);
}

#[test]
fn should_report_success_incomplete_and_unknown_scenarios() {
    for (scene, code) in [("completed", 0), ("incomplete", 3), ("not-a-scene", 2)] {
        let output = Command::new(env!("CARGO_BIN_EXE_carapana-prototype"))
            .args(["run", "--scenario", scene])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code));
    }
}

#[test]
fn should_list_shared_scenarios_and_mock_commands() {
    for command in ["scenarios", "doctor", "config", "sessions", "info"] {
        let output = Command::new(env!("CARGO_BIN_EXE_carapana-prototype"))
            .arg(command)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }
}
