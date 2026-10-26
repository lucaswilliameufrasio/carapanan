#![cfg(unix)]

use carapana_daemon::{
    CapturedProcessEnd, ProcessCaptureSettings, ProcessError, ProcessShutdownPolicy, ProcessSpec,
    ProcessSupervisor,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "carapana-captured-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn spec(&self, script: &str) -> ProcessSpec {
        ProcessSpec::new(
            "/usr/bin/python3",
            vec!["-I".into(), "-c".into(), script.into()],
            &self.0,
        )
    }
    fn group(&self) -> u32 {
        fs::read_to_string(self.0.join("group"))
            .unwrap()
            .parse()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn supervisor() -> ProcessSupervisor {
    ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapanan")).unwrap()
}
fn settings(total: u64, timeout: Duration) -> ProcessCaptureSettings {
    ProcessCaptureSettings::new(32, 32, total, timeout).unwrap()
}
fn assert_group_gone(id: u32) {
    use nix::{errno::Errno, sys::signal::killpg, unistd::Pid};
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if matches!(
            killpg(Pid::from_raw(i32::try_from(id).unwrap()), None),
            Err(Errno::ESRCH)
        ) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "captured fixture group remained alive"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn should_capture_private_binary_tails_and_the_actual_exit_code() {
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let script = "import os,sys\nopen('group','w').write(str(os.getpgrp()))\nfor _ in range(32):\n os.write(1,b'x'*4096); os.write(2,b'y'*4096)\nos.write(1,b'\\x00\\xff'); sys.exit(7)";
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture.spec(script),
            settings(512 * 1024, Duration::from_secs(5)),
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Exited);
    assert_eq!(output.exit_code(), Some(7));
    assert_eq!(output.stdout_bytes(), 32 * 4096 + 2);
    assert_eq!(output.stderr_bytes(), 32 * 4096);
    assert_eq!(&output.stdout_tail()[30..], &[0, 255]);
    assert_eq!(output.stderr_tail(), vec![b'y'; 32]);
    assert_group_gone(fixture.group());
    assert!(supervisor.owned_processes().unwrap().is_empty());
}

#[test]
fn should_terminate_the_owned_group_when_the_shared_output_budget_is_exhausted() {
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let output = supervisor.run_captured_owned_process("synthetic-session",
        fixture.spec("import os\nopen('group','w').write(str(os.getpgrp()))\nwhile True: os.write(1,b'x'*4096)"),
        settings(8193, Duration::from_secs(5)), || false).unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::CaptureLimit);
    assert_eq!(output.exit_code(), None);
    assert_eq!(output.stdout_bytes() + output.stderr_bytes(), 8193);
    assert!(output.stdout_tail().len() <= 32);
    assert_group_gone(fixture.group());
}

#[test]
fn should_wait_for_exit_or_timeout_after_both_pipes_close() {
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let script = "import os,time\nopen('group','w').write(str(os.getpgrp()))\nos.close(1); os.close(2); time.sleep(30)";
    let start = Instant::now();
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture.spec(script),
            settings(64, Duration::from_secs(5)),
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::TimedOut);
    assert_eq!(output.exit_code(), None);
    assert!(start.elapsed() < Duration::from_secs(12));
    assert_group_gone(fixture.group());
}

#[test]
fn should_cancel_and_collect_the_owned_group_without_requiring_eof() {
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let script = "import os,time\nopen('group','w').write(str(os.getpgrp()))\ntime.sleep(30)";
    let marker = fixture.0.join("group");
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture.spec(script),
            settings(64, Duration::from_secs(5)),
            || marker.exists(),
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Cancelled);
    assert_group_gone(fixture.group());
    assert!(supervisor.owned_processes().unwrap().is_empty());
}

#[test]
fn should_not_spawn_a_pre_cancelled_or_keep_running_capture() {
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let script = "open('should-not-exist','w').write('synthetic')";
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture.spec(script),
            settings(64, Duration::from_secs(5)),
            || true,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Cancelled);
    assert_eq!(output.stdout_bytes(), 0);
    assert!(!fixture.0.join("should-not-exist").exists());
    assert!(matches!(
        supervisor.run_captured_owned_process(
            "synthetic-session",
            fixture
                .spec(script)
                .with_shutdown_policy(ProcessShutdownPolicy::KeepRunning),
            settings(64, Duration::from_secs(5)),
            || false
        ),
        Err(ProcessError::InvalidProcessSpec)
    ));
    assert!(!fixture.0.join("should-not-exist").exists());
    assert!(supervisor.owned_processes().unwrap().is_empty());
}

#[test]
fn should_preserve_unrelated_processes_during_captured_group_cancellation() {
    use std::process::{Child, Command, Stdio};
    struct Unrelated(Child);
    impl Drop for Unrelated {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let mut unrelated = Unrelated(
        Command::new("/usr/bin/python3")
            .args(["-I", "-c", "import time; time.sleep(30)"])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let marker = fixture.0.join("group");
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture
                .spec("import os,time; open('group','w').write(str(os.getpgrp())); time.sleep(30)"),
            settings(64, Duration::from_secs(5)),
            || marker.exists(),
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Cancelled);
    assert_group_gone(fixture.group());
    assert!(unrelated.0.try_wait().unwrap().is_none());
}

#[test]
fn should_terminate_captured_group_after_its_private_controller_disconnects() {
    use std::{
        io::{Read, Write},
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    let fixture = Fixture::new();
    let mut host = Command::new(env!("CARGO_BIN_EXE_carapanan"))
        .arg("__carapana_internal_process_host_v1")
        .env_clear()
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut control = host.stdin.take().unwrap();
    let mut response = host.stdout.take().unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({
        "executable": "/usr/bin/python3",
        "arguments": ["-I", "-c", "import time; time.sleep(30)"],
        "working_directory": fixture.0,
        "shutdown_policy": "terminate",
        "capture": { "stdout_tail_bytes": 32, "stderr_tail_bytes": 32,
            "max_capture_bytes": 64, "timeout_ms": 30000 }
    }))
    .unwrap();
    control
        .write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
        .unwrap();
    control.write_all(&payload).unwrap();
    control.flush().unwrap();
    let mut size = [0; 4];
    response.read_exact(&mut size).unwrap();
    let mut frame = vec![0; u32::from_be_bytes(size) as usize];
    response.read_exact(&mut frame).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&frame).unwrap()["status"],
        "started"
    );
    let id = host.id();
    drop(control);
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "captured guardian did not handle controller EOF"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_group_gone(id);
}

#[test]
fn should_not_inherit_private_guardian_control_descriptors() {
    let fixture = Fixture::new();
    let database = fs::canonicalize(&fixture.0)
        .unwrap()
        .join("sessions.sqlite3");
    let artifacts = carapana_storage::ArtifactStore::open(&database).unwrap();
    let admissions = carapana_storage::PreparationAdmissionStore::open(
        &database,
        carapana_storage::PreparationAdmissionPolicy::new(1024, 1).unwrap(),
    )
    .unwrap();
    let mut supervisor = supervisor();
    let script = "import os\nleaked=[]\nfor fd in range(3,64):\n try: os.fstat(fd)\n except OSError: continue\n leaked.append(fd)\nassert not leaked, 'private guardian descriptor inherited'";
    let output = supervisor
        .run_captured_owned_process(
            "synthetic-session",
            fixture.spec(script),
            settings(4096, Duration::from_secs(5)),
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Exited);
    assert_eq!(
        output.exit_code(),
        Some(0),
        "target inherited private guardian descriptors"
    );
    assert_eq!(artifacts.usage().unwrap().reserved_bytes, 0);
    assert_eq!(admissions.usage().unwrap().reserved_bytes, 0);
}

#[test]
fn should_never_start_the_target_while_waiting_for_resource_admission() {
    use std::{
        io::{Read, Write},
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    let fixture = Fixture::new();
    let mut host = Command::new(env!("CARGO_BIN_EXE_carapanan"))
        .arg("__carapana_internal_process_host_v1")
        .env_clear()
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut control = host.stdin.take().unwrap();
    let mut response = host.stdout.take().unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({
        "executable":"/usr/bin/python3", "arguments":["-I","-c","open('not-admitted','w').write('forbidden')"],
        "working_directory":fixture.0, "shutdown_policy":"terminate", "wait_for_admission":true,
        "capture":{"stdout_tail_bytes":32,"stderr_tail_bytes":32,"max_capture_bytes":64,"timeout_ms":5000}
    })).unwrap();
    control
        .write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
        .unwrap();
    control.write_all(&payload).unwrap();
    control.flush().unwrap();
    let mut size = [0; 4];
    response.read_exact(&mut size).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(size) as usize];
    response.read_exact(&mut bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["status"],
        "ready_for_admission"
    );
    assert!(!fixture.0.join("not-admitted").exists());
    drop(control); // No Admit, so even controller EOF must not start the executable.
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!fixture.0.join("not-admitted").exists());
}
