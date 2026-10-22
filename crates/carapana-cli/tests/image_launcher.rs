#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use carapana_sandbox::{
    image_launcher::SandboxImageLaunch,
    runtime_manifest::{
        RuntimeFileRole, RuntimeFileSpec, RuntimeImage, RuntimeManifestPolicy,
        VerifiedRuntimeManifest,
    },
};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    os::{
        fd::OwnedFd,
        unix::{net::UnixDatagram, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn available() -> bool {
    match carapana_sandbox::probe_linux_bubblewrap() {
        Ok(_) => true,
        Err(error) => {
            assert!(
                std::env::var_os("CARAPANA_REQUIRE_BUBBLEWRAP").is_none(),
                "required Bubblewrap fixture unavailable: {error}"
            );
            false
        }
    }
}
fn policy() -> RuntimeManifestPolicy {
    RuntimeManifestPolicy::new(4, 32 * 1024 * 1024, 64 * 1024 * 1024).unwrap()
}
fn image(program: &str) -> RuntimeImage {
    // Fixed x86_64 test profile only: never discover runtime deps or derive production trust from
    // current hashes. Source aliases are canonicalized only for these explicitly named fixtures.
    let libc = ["/usr/lib/libc.so.6", "/usr/lib/x86_64-linux-gnu/libc.so.6"]
        .into_iter()
        .find(|path| Path::new(path).exists())
        .unwrap();
    let mut specs = Vec::new();
    for (source, target, role) in [
        (program, "/runtime/bin/program", RuntimeFileRole::Executable),
        (
            "/lib64/ld-linux-x86-64.so.2",
            "/lib64/ld-linux-x86-64.so.2",
            // The kernel executes PT_INTERP too; the explicit loader must be executable, not data.
            RuntimeFileRole::Executable,
        ),
        (
            libc,
            "/lib/x86_64-linux-gnu/libc.so.6",
            RuntimeFileRole::Support,
        ),
    ] {
        let source = fs::canonicalize(source).unwrap();
        let bytes = fs::read(&source).unwrap();
        assert!(bytes.len() < 32 * 1024 * 1024);
        specs.push(
            RuntimeFileSpec::new(
                source,
                PathBuf::from(target),
                role,
                Sha256::digest(bytes).into(),
            )
            .unwrap(),
        );
    }
    // Arch's glibc loader searches /usr/lib, Ubuntu's searches the multiarch directory. Bind the same
    // explicitly selected libc to both exact file destinations; neither profile mounts a directory.
    let source = fs::canonicalize(libc).unwrap();
    let hash = Sha256::digest(fs::read(&source).unwrap()).into();
    specs.push(
        RuntimeFileSpec::new(
            source,
            PathBuf::from("/usr/lib/libc.so.6"),
            RuntimeFileRole::Support,
            hash,
        )
        .unwrap(),
    );
    VerifiedRuntimeManifest::verify(specs, policy())
        .unwrap()
        .freeze(policy())
        .unwrap()
}

struct Helper(Child);
impl Drop for Helper {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(i32::try_from(self.0.id()).unwrap()),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = self.0.wait();
    }
}
fn run(program: &str, arguments: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    let launch = SandboxImageLaunch::new(
        image(program),
        PathBuf::from("/runtime/bin/program"),
        arguments.iter().map(OsString::from).collect(),
        1024 * 1024,
        policy(),
    )
    .unwrap();
    execute_launch(&launch)
}

fn execute_launch(launch: &SandboxImageLaunch<'_>) -> (i32, Vec<u8>, Vec<u8>) {
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut helper = Helper(
        Command::new(env!("CARGO_BIN_EXE_carapana"))
            .arg("__carapana_internal_image_launcher_v1")
            .env_clear()
            .env("CARAPANA_SYNTHETIC_OUTSIDE", "not-forwarded")
            .stdin(Stdio::from(OwnedFd::from(receiver)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    launch.send(&sender, deadline).unwrap();
    drop(sender);
    let status = loop {
        if let Some(status) = helper.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "synthetic image helper exceeded deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    helper
        .0
        .stdout
        .take()
        .unwrap()
        .take(8193)
        .read_to_end(&mut stdout)
        .unwrap();
    helper
        .0
        .stderr
        .take()
        .unwrap()
        .take(8193)
        .read_to_end(&mut stderr)
        .unwrap();
    assert!(stdout.len() <= 8192 && stderr.len() <= 8192);
    (status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn should_execute_only_the_declared_runtime_without_host_files_or_cgroupfs() {
    if !available() {
        return;
    }
    for arguments in [
        vec!["!", "-e", "/etc/passwd"],
        vec!["!", "-e", "/sys/fs/cgroup"],
        vec!["!", "-e", "/home"],
        vec!["!", "-w", "/"],
        vec!["!", "-w", "/workspace"],
        vec!["-w", "/tmp"],
        vec!["!", "-e", "/usr/bin/test"],
    ] {
        let (code, _, error) = run("/usr/bin/test", &arguments);
        assert_eq!(
            code,
            0,
            "minimal image check failed: {}",
            String::from_utf8_lossy(&error)
        );
    }
}

#[test]
fn should_isolate_network_namespace_and_close_the_setup_channel_before_target_exec() {
    if !available() {
        return;
    }
    let (code, output, error) = run("/usr/bin/readlink", &["/proc/self/ns/net"]);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&error));
    let host = fs::read_link("/proc/self/ns/net").unwrap();
    assert_ne!(
        String::from_utf8(output).unwrap().trim(),
        host.to_str().unwrap()
    );
    let (code, output, error) = run("/usr/bin/readlink", &["/proc/self/fd/0"]);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&error));
    assert_eq!(output, b"/dev/null\n");
}

#[test]
fn should_not_forward_ambient_environment_or_load_undeclared_executables() {
    if !available() {
        return;
    }
    let (code, output, error) = run("/usr/bin/env", &[]);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&error));
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("CARAPANA_SYNTHETIC_OUTSIDE"));
    for line in text.lines() {
        assert!(matches!(
            line,
            "LC_ALL=C" | "PATH=/runtime/bin:/usr/bin:/bin" | "PWD=/workspace"
        ));
    }
    assert!(matches!(
        SandboxImageLaunch::new(
            image("/usr/bin/true"),
            PathBuf::from("/usr/bin/undeclared"),
            Vec::new(),
            1024,
            policy()
        ),
        Err(carapana_sandbox::image_launcher::ImageLauncherError::InvalidRequest)
    ));
}

#[test]
fn should_drop_capabilities_enable_no_new_privileges_and_close_runtime_fds() {
    if !available() {
        return;
    }
    let (code, output, error) = run("/usr/bin/cat", &["/proc/self/status"]);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&error));
    let text = String::from_utf8(output).unwrap();
    assert!(text.lines().any(|line| line == "NoNewPrivs:\t1"));
    assert!(text.lines().any(|line| line == "CapEff:\t0000000000000000"));
    for fd in 3..9 {
        let path = format!("/proc/self/fd/{fd}");
        let (code, _, error) = run("/usr/bin/test", &["!", "-e", &path]);
        assert_eq!(
            code,
            0,
            "runtime fd {fd} leaked: {}",
            String::from_utf8_lossy(&error)
        );
    }
}

#[test]
fn should_reject_malformed_setup_without_starting_a_target_or_logging_payloads() {
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut helper = Helper(
        Command::new(env!("CARGO_BIN_EXE_carapana"))
            .arg("__carapana_internal_image_launcher_v1")
            .env_clear()
            .stdin(Stdio::from(OwnedFd::from(receiver)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    sender
        .send(b"synthetic malformed setup: must not be logged")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = helper.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(1));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut output = Vec::new();
    helper
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    helper
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert!(output.is_empty());
}

fn delegated_parent() -> Option<carapana_sandbox::delegated_cgroup::DelegatedCgroup> {
    let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
        assert!(
            std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
            "image guardian tests require an explicit delegated test parent"
        );
        return None;
    };
    Some(carapana_sandbox::delegated_cgroup::DelegatedCgroup::open(Path::new(&path)).unwrap())
}

#[test]
fn should_run_the_minimal_image_under_the_admitted_guardian_and_release_the_scope() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    let mut group = parent
        .prepare(
            carapana_sandbox::delegated_cgroup::CgroupLimits::new(128 * 1024 * 1024, 32).unwrap(),
        )
        .unwrap();
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/test"),
        PathBuf::from("/runtime/bin/program"),
        vec!["!".into(), "-e".into(), "/sys/fs/cgroup".into()],
        1024 * 1024,
        policy(),
    )
    .unwrap();
    let mut supervisor =
        carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana")).unwrap();
    let output = supervisor
        .run_captured_image_process(
            "synthetic-image-session",
            &launch,
            carapana_daemon::ProcessCaptureSettings::new(
                4096,
                1024,
                16384,
                Duration::from_secs(10),
            )
            .unwrap(),
            &mut group,
            || false,
        )
        .unwrap();
    assert_eq!(output.end(), carapana_daemon::CapturedProcessEnd::Exited);
    assert_eq!(
        output.exit_code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(output.stderr_tail())
    );
    group.verify_empty().unwrap();
    group.release().unwrap();
    assert!(supervisor.owned_processes().unwrap().is_empty());
}

#[test]
fn should_cancel_the_sandbox_image_and_collect_bubblewrap_and_its_target() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    let mut group = parent
        .prepare(
            carapana_sandbox::delegated_cgroup::CgroupLimits::new(128 * 1024 * 1024, 32).unwrap(),
        )
        .unwrap();
    let unique_sleep = format!(
        "30.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/sleep"),
        PathBuf::from("/runtime/bin/program"),
        vec![unique_sleep.clone().into()],
        1024 * 1024,
        policy(),
    )
    .unwrap();
    let mut supervisor =
        carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana")).unwrap();
    let start = Instant::now();
    let mut observed_target = false;
    let output = supervisor
        .run_captured_image_process(
            "synthetic-image-session",
            &launch,
            carapana_daemon::ProcessCaptureSettings::new(
                4096,
                1024,
                16384,
                Duration::from_secs(10),
            )
            .unwrap(),
            &mut group,
            || {
                observed_target |= observed_scoped_target(&unique_sleep);
                observed_target || start.elapsed() > Duration::from_secs(8)
            },
        )
        .unwrap();
    assert_eq!(output.end(), carapana_daemon::CapturedProcessEnd::Cancelled);
    assert!(
        observed_target,
        "cancellation must follow real target exec, not just helper startup"
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    group.verify_empty().unwrap();
    group.release().unwrap();
}

fn observed_scoped_target(unique_argument: &str) -> bool {
    // Observe only this explicitly delegated test parent's own job groups. Never discover, signal,
    // or inspect unrelated host processes. A unique synthetic argv proves the final target exec'd.
    let root = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT").unwrap();
    for entry in fs::read_dir(root).unwrap().take(64) {
        let entry = entry.unwrap();
        if !entry
            .file_name()
            .as_encoded_bytes()
            .starts_with(b"carapana-job-")
        {
            continue;
        }
        let Ok(file) = fs::File::open(entry.path().join("cgroup.procs")) else {
            continue;
        };
        let mut pids = String::new();
        if file.take(4096).read_to_string(&mut pids).is_err() {
            continue;
        }
        for pid in pids.lines() {
            let Ok(pid) = pid.parse::<u32>() else {
                continue;
            };
            let Ok(file) = fs::File::open(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(4096).read_to_end(&mut bytes).is_err() {
                continue;
            }
            let mut argv = bytes.split(|byte| *byte == 0);
            if argv.next() == Some(b"/runtime/bin/program".as_slice())
                && argv.next() == Some(unique_argument.as_bytes())
            {
                return true;
            }
        }
    }
    false
}

struct SnapshotFixture(PathBuf);
impl SnapshotFixture {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "carapana-image-workspace-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::write(root.join("input.txt"), b"selected synthetic bytes\n").unwrap();
        fs::write(root.join("unselected.txt"), b"not selected\n").unwrap();
        Self(root)
    }
    fn database(&self) -> PathBuf {
        self.0.join("data/sessions.sqlite3")
    }
    fn metadata(&self) -> carapana_storage::WorkspaceMetadata {
        carapana_storage::WorkspaceMetadata::capture(&self.0).unwrap()
    }
}
impl Drop for SnapshotFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn should_mount_only_reserved_selected_bytes_and_retain_quota_until_launch_drop() {
    if !available() {
        return;
    }
    use carapana_sandbox::selected_snapshots::{
        ReservedSelectedSnapshotBundle, SelectedSnapshotPolicy,
    };
    let fixture = SnapshotFixture::new();
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let selected = ReservedSelectedSnapshotBundle::prepare(
        &mut store,
        &fixture.metadata(),
        &[PathBuf::from("input.txt")],
        SelectedSnapshotPolicy::new(1, 1024).unwrap(),
        1000,
    )
    .unwrap();
    let staging = selected.staging_root().to_owned();
    let sealed = selected.seal_for_launch().unwrap();
    assert_eq!(
        sealed.source_identity(),
        &fixture.metadata().directory_identity()
    );
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/cat"),
        PathBuf::from("/runtime/bin/program"),
        vec![
            "/workspace/input.txt".into(),
            "/workspace/unselected.txt".into(),
        ],
        1024 * 1024,
        policy(),
    )
    .unwrap()
    .with_workspace(sealed)
    .unwrap();
    fs::write(
        fixture.0.join("input.txt"),
        b"external edit after selection\n",
    )
    .unwrap();
    let observer = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    assert_eq!(observer.usage().unwrap().reserved_bytes, 1024);
    let (code, output, error) = execute_launch(&launch);
    assert_eq!(code, 1, "unselected file must not be present");
    assert_eq!(output, b"selected synthetic bytes\n");
    assert!(String::from_utf8_lossy(&error).contains("/workspace/unselected.txt"));
    assert_eq!(
        fs::read(fixture.0.join("input.txt")).unwrap(),
        b"external edit after selection\n"
    );
    assert_eq!(observer.usage().unwrap().reserved_bytes, 1024);
    drop(launch);
    assert!(!staging.exists());
    assert_eq!(observer.usage().unwrap().reserved_bytes, 0);
}

#[test]
fn should_transfer_the_maximum_selection_in_bounded_batches_under_the_guardian() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    use carapana_sandbox::selected_snapshots::{
        ReservedSelectedSnapshotBundle, SelectedSnapshotPolicy,
    };
    let fixture = SnapshotFixture::new();
    let paths: Vec<_> = (0..256)
        .map(|index| PathBuf::from(format!("file-{index:03}")))
        .collect();
    for path in &paths {
        fs::write(fixture.0.join(path), b"x").unwrap();
    }
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let selected = ReservedSelectedSnapshotBundle::prepare(
        &mut store,
        &fixture.metadata(),
        &paths,
        SelectedSnapshotPolicy::new(256, 1024).unwrap(),
        1000,
    )
    .unwrap()
    .seal_for_launch()
    .unwrap();
    assert_eq!(selected.file_count(), 256);
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/cat"),
        PathBuf::from("/runtime/bin/program"),
        vec!["/workspace/file-000".into(), "/workspace/file-255".into()],
        1024 * 1024,
        policy(),
    )
    .unwrap()
    .with_workspace(selected)
    .unwrap();
    let mut group = parent
        .prepare(
            carapana_sandbox::delegated_cgroup::CgroupLimits::new(128 * 1024 * 1024, 32).unwrap(),
        )
        .unwrap();
    let output = carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana"))
        .unwrap()
        .run_captured_image_process(
            "synthetic-selected-session",
            &launch,
            carapana_daemon::ProcessCaptureSettings::new(
                4096,
                1024,
                16384,
                Duration::from_secs(10),
            )
            .unwrap(),
            &mut group,
            || false,
        )
        .unwrap();
    assert_eq!(
        output.exit_code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(output.stderr_tail())
    );
    assert_eq!(output.stdout_tail(), b"xx");
    group.verify_empty().unwrap();
    group.release().unwrap();
    drop(launch);
    assert_eq!(store.usage().unwrap().reserved_bytes, 0);
}

#[test]
fn should_reject_changed_staging_before_sending_any_workspace_bytes() {
    use carapana_sandbox::selected_snapshots::{
        ReservedSelectedSnapshotBundle, SelectedSnapshotPolicy,
    };
    let fixture = SnapshotFixture::new();
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let selected = ReservedSelectedSnapshotBundle::prepare(
        &mut store,
        &fixture.metadata(),
        &[PathBuf::from("input.txt")],
        SelectedSnapshotPolicy::new(1, 1024).unwrap(),
        1000,
    )
    .unwrap();
    let staging = selected.staging_root().to_owned();
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/true"),
        PathBuf::from("/runtime/bin/program"),
        Vec::new(),
        1024,
        policy(),
    )
    .unwrap()
    .with_workspace(selected.seal_for_launch().unwrap())
    .unwrap();
    fs::write(staging.join("input.txt"), b"tampered staging\n").unwrap();
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    receiver.set_nonblocking(true).unwrap();
    assert!(matches!(
        launch.send(&sender, Instant::now() + Duration::from_secs(1)),
        Err(carapana_sandbox::image_launcher::ImageLauncherError::RuntimeRejected)
    ));
    assert_eq!(
        receiver.recv(&mut [0_u8; 1]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(launch);
    assert_eq!(store.usage().unwrap().reserved_bytes, 0);
}

fn reviewed_proposal<'store>(
    fixture: &SnapshotFixture,
    store: &'store mut carapana_storage::ArtifactStore,
    program: &str,
    arguments: Vec<OsString>,
    capture: carapana_daemon::ProcessCaptureSettings,
) -> (carapana_daemon::SandboxExecutionProposal<'store>, PathBuf) {
    use carapana_sandbox::selected_snapshots::{
        ReservedSelectedSnapshotBundle, SelectedSnapshotPolicy,
    };
    let selected = ReservedSelectedSnapshotBundle::prepare(
        store,
        &fixture.metadata(),
        &[PathBuf::from("input.txt")],
        SelectedSnapshotPolicy::new(1, 1024).unwrap(),
        1000,
    )
    .unwrap();
    let staging = selected.staging_root().to_owned();
    let launch = SandboxImageLaunch::new(
        image(program),
        PathBuf::from("/runtime/bin/program"),
        arguments,
        1024 * 1024,
        policy(),
    )
    .unwrap()
    .with_workspace(selected.seal_for_launch().unwrap())
    .unwrap();
    let limits = carapana_daemon::SandboxExecutionLimits::new(
        capture,
        carapana_sandbox::delegated_cgroup::CgroupLimits::new(128 * 1024 * 1024, 32).unwrap(),
    )
    .unwrap();
    let proposal = carapana_daemon::SandboxExecutionProposal::new(
        carapana_core::trust::WorkspaceId::new("review-workspace").unwrap(),
        carapana_core::trust::TaskId::new("review-task").unwrap(),
        &fixture.metadata(),
        launch,
        limits,
    )
    .unwrap();
    (proposal, staging)
}

#[test]
fn should_enforce_the_exact_approved_image_snapshot_and_output_budget() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    use carapana_core::{
        command_approval::{ApprovalSubject, CommandApprovalError},
        trust::{TaskId, WorkspaceId},
    };
    let fixture = SnapshotFixture::new();
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let capture =
        carapana_daemon::ProcessCaptureSettings::new(4, 4, 8, Duration::from_secs(10)).unwrap();
    let (proposal, staging) = reviewed_proposal(
        &fixture,
        &mut store,
        "/usr/bin/cat",
        vec!["/workspace/input.txt".into()],
        capture,
    );
    assert_eq!(
        proposal.launch().program(),
        Path::new("/runtime/bin/program")
    );
    assert_eq!(proposal.limits().capture().max_capture_bytes(), 8);
    let mut request = proposal.request_approval(100, 10).unwrap();
    let other_request = proposal.request_approval(100, 10).unwrap();
    assert!(matches!(
        request.approve_by_operator(
            &other_request.id(),
            proposal.workspace(),
            proposal.task(),
            11
        ),
        Err(CommandApprovalError::WrongRequest)
    ));
    drop(other_request);
    let token = request
        .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
        .unwrap();
    fs::write(fixture.0.join("input.txt"), b"external source edit\n").unwrap();
    let source = fixture.metadata();
    let workspace = WorkspaceId::new("review-workspace").unwrap();
    let task = TaskId::new("review-task").unwrap();
    let mut group = parent.prepare(proposal.limits().resources()).unwrap();
    let output = carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana"))
        .unwrap()
        .run_approved_image_process(
            token,
            carapana_daemon::SandboxExecutionScope {
                session_id: "synthetic-approved",
                workspace_id: &workspace,
                task: &task,
                source: &source,
            },
            &mut group,
            || 12,
            || false,
        )
        .unwrap();
    assert_eq!(
        output.end(),
        carapana_daemon::CapturedProcessEnd::CaptureLimit
    );
    assert_eq!(output.stdout_bytes(), 8);
    assert_eq!(output.stdout_tail(), b"cted");
    assert_eq!(
        fs::read(fixture.0.join("input.txt")).unwrap(),
        b"external source edit\n"
    );
    group.verify_empty().unwrap();
    group.release().unwrap();
    assert!(matches!(
        request.approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 12),
        Err(CommandApprovalError::NotPending)
    ));
    drop(request);
    drop(proposal);
    assert!(!staging.exists());
    assert_eq!(store.usage().unwrap().reserved_bytes, 0);
}

#[test]
fn should_reject_stale_scope_staging_and_limits_without_admitting_a_guardian() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    use carapana_core::{
        command_approval::{ApprovalSubject, CommandApprovalError},
        trust::{TaskId, WorkspaceId},
    };
    use carapana_daemon::SandboxExecutionError;
    for case in 0..8 {
        let fixture = SnapshotFixture::new();
        let other = SnapshotFixture::new();
        let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
        let capture =
            carapana_daemon::ProcessCaptureSettings::new(32, 32, 1024, Duration::from_secs(10))
                .unwrap();
        let (proposal, staging) =
            reviewed_proposal(&fixture, &mut store, "/usr/bin/true", Vec::new(), capture);
        let mut request = Some(proposal.request_approval(100, 10).unwrap());
        let id = request.as_ref().unwrap().id();
        let token = request
            .as_mut()
            .unwrap()
            .approve_by_operator(&id, proposal.workspace(), proposal.task(), 11)
            .unwrap();
        if case == 1 {
            request.as_mut().unwrap().invalidate();
        }
        if case == 2 {
            drop(request.take());
        }
        if case == 7 {
            fs::write(staging.join("input.txt"), b"changed staging").unwrap();
        }
        let source = if case == 5 {
            other.metadata()
        } else {
            fixture.metadata()
        };
        let workspace = WorkspaceId::new(if case == 3 {
            "other"
        } else {
            "review-workspace"
        })
        .unwrap();
        let task = TaskId::new(if case == 4 { "other" } else { "review-task" }).unwrap();
        let limits = if case == 6 {
            carapana_sandbox::delegated_cgroup::CgroupLimits::new(64 * 1024 * 1024, 32).unwrap()
        } else {
            proposal.limits().resources()
        };
        let mut group = parent.prepare(limits).unwrap();
        let result = carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana"))
            .unwrap()
            .run_approved_image_process(
                token,
                carapana_daemon::SandboxExecutionScope {
                    session_id: "synthetic-approved",
                    workspace_id: &workspace,
                    task: &task,
                    source: &source,
                },
                &mut group,
                || if case == 0 { 100 } else { 12 },
                || false,
            );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("invalid approval reached execution"),
        };
        match case {
            0 => assert!(matches!(
                error,
                SandboxExecutionError::Approval(CommandApprovalError::Expired)
            )),
            1 | 2 => assert!(matches!(
                error,
                SandboxExecutionError::Approval(CommandApprovalError::Invalidated)
            )),
            3 => assert!(matches!(
                error,
                SandboxExecutionError::Approval(CommandApprovalError::WrongWorkspace)
            )),
            4 => assert!(matches!(
                error,
                SandboxExecutionError::Approval(CommandApprovalError::WrongTask)
            )),
            5 => assert!(matches!(
                error,
                SandboxExecutionError::PhysicalWorkspaceMismatch
            )),
            6 => assert!(matches!(error, SandboxExecutionError::LimitMismatch)),
            _ => assert!(matches!(error, SandboxExecutionError::StagingChanged)),
        }
        group.verify_ready_for_admission().unwrap();
        group.release().unwrap();
        drop(request);
        drop(proposal);
        assert_eq!(store.usage().unwrap().reserved_bytes, 0);
    }
}

#[test]
fn should_revoke_an_approved_image_after_observing_the_target_running() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    use carapana_core::command_approval::{ApprovalSubject, CommandApprovalError};
    let fixture = SnapshotFixture::new();
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let argument = format!(
        "30.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let capture =
        carapana_daemon::ProcessCaptureSettings::new(32, 32, 1024, Duration::from_secs(10))
            .unwrap();
    let (proposal, _) = reviewed_proposal(
        &fixture,
        &mut store,
        "/usr/bin/sleep",
        vec![argument.clone().into()],
        capture,
    );
    let mut request = proposal.request_approval(100, 10).unwrap();
    let token = request
        .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
        .unwrap();
    let mut group = parent.prepare(proposal.limits().resources()).unwrap();
    let source = fixture.metadata();
    let start = Instant::now();
    let mut observed = false;
    let result = carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana"))
        .unwrap()
        .run_approved_image_process(
            token,
            carapana_daemon::SandboxExecutionScope {
                session_id: "synthetic-revocation",
                workspace_id: proposal.workspace(),
                task: proposal.task(),
                source: &source,
            },
            &mut group,
            || 12,
            || {
                observed |= observed_scoped_target(&argument);
                if observed {
                    request.invalidate();
                }
                start.elapsed() > Duration::from_secs(8)
            },
        );
    assert!(observed, "revocation must follow target exec");
    assert!(matches!(
        result,
        Err(carapana_daemon::SandboxExecutionError::Approval(
            CommandApprovalError::Invalidated
        ))
    ));
    group.verify_empty().unwrap();
    group.release().unwrap();
}

#[test]
fn should_recheck_approval_expiry_after_ready_but_before_admission() {
    let Some(mut parent) = delegated_parent() else {
        return;
    };
    use carapana_core::command_approval::{ApprovalSubject, CommandApprovalError};
    let fixture = SnapshotFixture::new();
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let capture =
        carapana_daemon::ProcessCaptureSettings::new(32, 32, 1024, Duration::from_secs(10))
            .unwrap();
    let (proposal, _) =
        reviewed_proposal(&fixture, &mut store, "/usr/bin/true", Vec::new(), capture);
    let mut request = proposal.request_approval(100, 10).unwrap();
    let token = request
        .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
        .unwrap();
    let mut group = parent.prepare(proposal.limits().resources()).unwrap();
    let source = fixture.metadata();
    let now = std::cell::Cell::new(12);
    let checks = std::cell::Cell::new(0);
    let result = carapana_daemon::ProcessSupervisor::new(env!("CARGO_BIN_EXE_carapana"))
        .unwrap()
        .run_approved_image_process(
            token,
            carapana_daemon::SandboxExecutionScope {
                session_id: "synthetic-expiry",
                workspace_id: proposal.workspace(),
                task: proposal.task(),
                source: &source,
            },
            &mut group,
            || now.get(),
            || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    now.set(100);
                }
                false
            },
        );
    assert!(
        checks.get() >= 2,
        "fixture must reach ReadyForAdmission before advancing its clock"
    );
    assert!(matches!(
        result,
        Err(carapana_daemon::SandboxExecutionError::Approval(
            CommandApprovalError::Expired
        ))
    ));
    group.verify_ready_for_admission().unwrap();
    group.release().unwrap();
}

#[test]
fn should_require_a_reserved_workspace_from_the_reviewed_physical_source() {
    use carapana_core::trust::{TaskId, WorkspaceId};
    use carapana_daemon::{
        SandboxExecutionError, SandboxExecutionLimits, SandboxExecutionProposal,
    };
    let fixture = SnapshotFixture::new();
    let capture =
        carapana_daemon::ProcessCaptureSettings::new(32, 32, 1024, Duration::from_secs(10))
            .unwrap();
    let limits = SandboxExecutionLimits::new(
        capture,
        carapana_sandbox::delegated_cgroup::CgroupLimits::new(128 * 1024 * 1024, 32).unwrap(),
    )
    .unwrap();
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/true"),
        PathBuf::from("/runtime/bin/program"),
        Vec::new(),
        1024,
        policy(),
    )
    .unwrap();
    assert!(matches!(
        SandboxExecutionProposal::new(
            WorkspaceId::new("w").unwrap(),
            TaskId::new("t").unwrap(),
            &fixture.metadata(),
            launch,
            limits
        ),
        Err(SandboxExecutionError::MissingReservedWorkspace)
    ));
    let mut store = carapana_storage::ArtifactStore::open(fixture.database()).unwrap();
    let selected = carapana_sandbox::selected_snapshots::ReservedSelectedSnapshotBundle::prepare(
        &mut store,
        &fixture.metadata(),
        &[PathBuf::from("input.txt")],
        carapana_sandbox::selected_snapshots::SelectedSnapshotPolicy::new(1, 1024).unwrap(),
        1000,
    )
    .unwrap()
    .seal_for_launch()
    .unwrap();
    let launch = SandboxImageLaunch::new(
        image("/usr/bin/true"),
        PathBuf::from("/runtime/bin/program"),
        Vec::new(),
        1024,
        policy(),
    )
    .unwrap()
    .with_workspace(selected)
    .unwrap();
    let other = SnapshotFixture::new();
    assert!(matches!(
        SandboxExecutionProposal::new(
            WorkspaceId::new("w").unwrap(),
            TaskId::new("t").unwrap(),
            &other.metadata(),
            launch,
            limits
        ),
        Err(SandboxExecutionError::PhysicalWorkspaceMismatch)
    ));
    assert_eq!(store.usage().unwrap().reserved_bytes, 0);
}
