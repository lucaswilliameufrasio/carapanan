#![cfg(unix)]

use std::process::{Command, Stdio};

#[test]
fn should_run_a_synthetic_sandbox_probe_without_credentials_or_a_daemon() {
    let output = Command::new(env!("CARGO_BIN_EXE_carapanan"))
        .arg("sandbox-probe")
        .env_clear()
        .env("TMPDIR", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stdout.contains("/home/"));
    assert!(!stderr.contains("Traceback"));
    #[cfg(not(target_os = "linux"))]
    assert!(
        !output.status.success(),
        "unsupported platform reported enforcement"
    );
    if output.status.success() {
        assert_eq!(
            stdout,
            "Synthetic Linux sandbox fixture verified. User commands and tools remain disabled.\n"
        );
        assert!(stderr.is_empty());
    } else {
        assert!(stdout.is_empty());
        assert!(matches!(
            stderr.trim(),
            "carapanan: sandbox probe is supported only on Linux"
                | "carapanan: Bubblewrap is unavailable"
                | "carapanan: sandbox enforcement was not verified"
                | "carapanan: Bubblewrap executable is not a private-safe system executable"
        ));
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("CARAPANA_REQUIRE_BUBBLEWRAP").is_some() {
        assert!(
            output.status.success(),
            "required synthetic probe failed: {stderr}"
        );
    }
}

#[test]
fn should_not_accept_commands_or_workspace_paths_in_the_sandbox_probe() {
    let output = Command::new(env!("CARGO_BIN_EXE_carapanan"))
        .args(["sandbox-probe", "--", "/bin/true"])
        .env_clear()
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
