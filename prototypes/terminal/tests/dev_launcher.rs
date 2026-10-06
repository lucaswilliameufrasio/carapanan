#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn should_launch_dev_with_a_bsd_compatible_template_and_stop_web_on_exit() {
    let directory = std::env::temp_dir().join(format!("carapana-dev-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let script = |name: &str, content: &str| {
        let path = directory.join(name);
        fs::write(&path, content).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    };
    // GNU mktemp accepts a suffix after Xs; BSD/macOS requires trailing Xs.
    // This shim checks that contract, then uses the real local filesystem utility.
    script(
        "mktemp",
        "#!/bin/bash\ncase \"$1\" in *XXXXXX) exec /usr/bin/mktemp \"$@\" ;; *) echo 'BSD mktemp: template must end in XXXXXX' >&2; exit 1 ;; esac\n",
    );
    script(
        "pnpm",
        "#!/bin/bash\ntrap 'echo stopped > \"$TEST_DIRECTORY/stopped\"; exit 0' TERM\nprintf '%s:%s' \"$HOST\" \"$PORT\" > \"$TEST_DIRECTORY/network\"\necho ready > \"$TEST_DIRECTORY/ready\"\nsleep 30 &\nwait\n",
    );
    script(
        "cargo",
        "#!/bin/bash\nfor i in {1..100}; do [ -f \"$TEST_DIRECTORY/ready\" ] && exit \"$TEST_EXIT_CODE\"; sleep .02; done\nexit 99\n",
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (code, host, port, expected) in [
        (0, "", "", "127.0.0.1:5173"),
        (7, "", "", "127.0.0.1:5173"),
        (0, "0.0.0.0", "5180", "0.0.0.0:5180"),
        (7, "0.0.0.0", "5180", "0.0.0.0:5180"),
    ] {
        let _ = fs::remove_file(directory.join("ready"));
        let _ = fs::remove_file(directory.join("stopped"));
        let output = Command::new("make")
            .arg("dev")
            .current_dir(&root)
            .env(
                "PATH",
                format!("{}:{}", directory.display(), std::env::var("PATH").unwrap()),
            )
            .env("TMPDIR", &directory)
            .env("TEST_DIRECTORY", &directory)
            .env("TEST_EXIT_CODE", code.to_string())
            .env("HOST", host)
            .env("PORT", port)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            code == 0,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("Web: http://{expected}")),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(directory.join("network")).unwrap(),
            expected
        );
        assert!(
            directory.join("stopped").exists(),
            "web must receive TERM on TUI success or failure"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}
