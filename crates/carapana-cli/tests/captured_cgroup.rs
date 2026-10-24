#![cfg(target_os = "linux")]

use carapana_daemon::{
    CapturedProcessEnd, ProcessCaptureSettings, ProcessError, ProcessSpec, ProcessSupervisor,
};
use carapana_sandbox::delegated_cgroup::{CgroupLimits, DelegatedCgroup};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "carapana-scoped-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
    fn spec(&self, script: &str) -> ProcessSpec {
        ProcessSpec::new(
            "/usr/bin/python3",
            vec!["-I".into(), "-c".into(), script.into()],
            &self.0,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn configured_parent() -> Option<DelegatedCgroup> {
    let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
        assert!(
            std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
            "real scoped guardian test requires explicit parent"
        );
        return None;
    };
    Some(DelegatedCgroup::open(Path::new(&path)).unwrap())
}
fn settings(total: u64) -> ProcessCaptureSettings {
    ProcessCaptureSettings::new(2048, 256, total, Duration::from_secs(5)).unwrap()
}
fn supervisor() -> ProcessSupervisor {
    ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana")).unwrap()
}

#[test]
fn should_start_the_target_only_inside_the_admitted_guardian_cgroup() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    let fixture = Fixture::new();
    let mut group = parent
        .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let output = supervisor()
        .run_captured_resource_scoped_process(
            "synthetic",
        fixture.spec("import os; print(open('/proc/self/cgroup').read()); print(os.readlink('/proc/self/fd/2'))"),
            settings(8192),
            &mut group,
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Exited);
    assert_eq!(output.exit_code(), Some(0));
    let identity = std::str::from_utf8(output.stdout_tail()).unwrap();
    assert!(identity.contains("/carapana-job-"));
    assert!(!identity.contains("/runner"));
    assert!(
        identity.contains("pipe:["),
        "target stderr must not inherit the guardian kill switch"
    );
    group.verify_empty().unwrap();
    group.release().unwrap();
}

#[test]
fn should_collect_a_descendant_that_escaped_the_guardians_process_group() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    let fixture = Fixture::new();
    let mut group = parent
        .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let script = "import os,time\nr,w=os.pipe()\npid=os.fork()\nif pid==0:\n os.close(r)\n os.setsid()\n assert os.getpgrp()==os.getpid()\n os.write(w,b'1');os.close(w)\n os.close(1);os.close(2)\n time.sleep(30)\n os._exit(0)\nos.close(w)\nassert os.read(r,1)==b'1'\nprint('escaped-confirmed')";
    let output = supervisor()
        .run_captured_resource_scoped_process(
            "synthetic",
            fixture.spec(script),
            settings(8192),
            &mut group,
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), CapturedProcessEnd::Exited);
    assert_eq!(output.stdout_tail(), b"escaped-confirmed\n");
    // Without cgroup cleanup the setsid descendant would remain alive for 30 seconds.
    group.verify_empty().unwrap();
    group.release().unwrap();
}

#[test]
fn should_reject_reusing_a_scope_before_starting_a_second_target() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    let fixture = Fixture::new();
    let mut supervisor = supervisor();
    let mut group = parent
        .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let first = supervisor
        .run_captured_resource_scoped_process(
            "synthetic",
            fixture.spec("pass"),
            settings(8192),
            &mut group,
            || false,
        )
        .unwrap();
    assert_eq!(first.exit_code(), Some(0));
    assert!(matches!(
        supervisor.run_captured_resource_scoped_process(
            "synthetic",
            fixture.spec("open('forbidden','w').write('not admitted')"),
            settings(8192),
            &mut group,
            || false
        ),
        Err(ProcessError::ResourceAdmissionFailed)
    ));
    assert!(!fixture.0.join("forbidden").exists());
    group.release().unwrap();
}

#[test]
fn should_not_fall_back_outside_the_scope_when_memory_kills_the_guardian() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    let fixture = Fixture::new();
    let mut group = parent
        .prepare(CgroupLimits::new(32 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let script = "value=bytearray(128*1024*1024)\nfor offset in range(0,len(value),4096): value[offset]=1\nopen('escaped-budget','w').write('not allowed')";
    assert!(
        supervisor()
            .run_captured_resource_scoped_process(
                "synthetic",
                fixture.spec(script),
                settings(8192),
                &mut group,
                || false
            )
            .is_err()
    );
    assert!(!fixture.0.join("escaped-budget").exists());
    group.verify_empty().unwrap();
    group.release().unwrap();
}

fn write_frame(writer: &mut impl std::io::Write, value: &serde_json::Value) {
    let bytes = serde_json::to_vec(value).unwrap();
    writer
        .write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes())
        .unwrap();
    writer.write_all(&bytes).unwrap();
    writer.flush().unwrap();
}
fn read_frame(reader: &mut impl std::io::Read) -> serde_json::Value {
    let mut size = [0; 4];
    reader.read_exact(&mut size).unwrap();
    assert!(u32::from_be_bytes(size) <= 64 * 1024);
    let mut bytes = vec![0; u32::from_be_bytes(size) as usize];
    reader.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn should_kill_the_resource_scope_on_controller_loss_even_with_a_setsid_descendant() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        time::Instant,
    };
    let fixture = Fixture::new();
    let mut group = parent
        .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let mut host = Command::new(env!("CARGO_BIN_EXE_carapana"))
        .arg("__carapana_internal_process_host_v1")
        .env_clear()
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(group.guardian_kill_stdio().unwrap())
        .spawn()
        .unwrap();
    let mut control = host.stdin.take().unwrap();
    let mut response = host.stdout.take().unwrap();
    let script = "import os,time\npid=os.fork()\nif pid==0:\n os.setsid()\n open('escaped-ready','w').write('ready')\ntime.sleep(30)";
    write_frame(
        &mut control,
        &serde_json::json!({"executable":"/usr/bin/python3","arguments":["-I","-c",script],
        "working_directory":fixture.0,"shutdown_policy":"terminate","wait_for_admission":true,
        "capture":{"stdout_tail_bytes":32,"stderr_tail_bytes":32,"max_capture_bytes":64,"timeout_ms":30000}}),
    );
    assert_eq!(read_frame(&mut response)["status"], "ready_for_admission");
    group.admit_blocked_child(&mut host).unwrap();
    write_frame(&mut control, &serde_json::json!("admit"));
    assert_eq!(read_frame(&mut response)["status"], "started");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.0.join("escaped-ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(control);
    while host.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    // No explicit parent-side terminate: the guardian's pinned kill switch must do the cleanup.
    group.verify_empty().unwrap();
    group.release().unwrap();
}

#[test]
fn should_reject_an_admit_message_without_actual_kernel_membership() {
    let Some(mut parent) = configured_parent() else {
        return;
    };
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        time::Instant,
    };
    let fixture = Fixture::new();
    let group = parent
        .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
        .unwrap();
    let mut host = Command::new(env!("CARGO_BIN_EXE_carapana"))
        .arg("__carapana_internal_process_host_v1")
        .env_clear()
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(group.guardian_kill_stdio().unwrap())
        .spawn()
        .unwrap();
    let mut control = host.stdin.take().unwrap();
    let mut response = host.stdout.take().unwrap();
    write_frame(
        &mut control,
        &serde_json::json!({"executable":"/usr/bin/python3","arguments":["-I","-c","open('forbidden','w').write('not admitted')"],
        "working_directory":fixture.0,"shutdown_policy":"terminate","wait_for_admission":true,
        "capture":{"stdout_tail_bytes":32,"stderr_tail_bytes":32,"max_capture_bytes":64,"timeout_ms":5000}}),
    );
    assert_eq!(read_frame(&mut response)["status"], "ready_for_admission");
    // Deliberately do not move the guardian into the cgroup; a message alone is not enforcement.
    write_frame(&mut control, &serde_json::json!("admit"));
    drop(control);
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!fixture.0.join("forbidden").exists());
    assert_eq!(
        host.wait().unwrap().code(),
        Some(1),
        "private guardian errors must not log to its reserved control fd or abort"
    );
    group.verify_empty().unwrap();
    group.release().unwrap();
}
