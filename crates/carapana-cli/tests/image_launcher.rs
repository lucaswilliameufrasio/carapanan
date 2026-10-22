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
