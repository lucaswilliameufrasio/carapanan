//! Synthetic enforcement probes only: no user commands, workspace mounts, or runtime wiring.
//!
//! A successful probe is evidence about this fixed fixture on the current host, not authorization
//! to execute tools or proof of secret protection in an arbitrary workspace.

pub mod selected_snapshots;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxProbeError {
    UnsupportedPlatform,
    BackendUnavailable,
    UnsafeBackend,
    FixtureUnavailable,
    SpawnFailed,
    TimedOut,
    OutputLimitExceeded,
    EnforcementNotVerified,
}

impl std::fmt::Display for SandboxProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "sandbox probe is supported only on Linux",
            Self::BackendUnavailable => "Bubblewrap is unavailable",
            Self::UnsafeBackend => "Bubblewrap executable is not a private-safe system executable",
            Self::FixtureUnavailable => "synthetic sandbox fixture could not be prepared",
            Self::SpawnFailed => "sandbox probe could not be started",
            Self::TimedOut => "sandbox probe exceeded its deadline",
            Self::OutputLimitExceeded => "sandbox probe exceeded its output budget",
            Self::EnforcementNotVerified => "sandbox enforcement was not verified",
        })
    }
}

impl std::error::Error for SandboxProbeError {}

/// Evidence for the fixed read-only fixture, not a reusable execution permission.
#[derive(Debug, PartialEq, Eq)]
pub struct SandboxProbeEvidence {
    _private: (),
}

/// Run a fixed synthetic probe with bounded stdout/stderr and a five-second deadline.
///
/// Missing/restricted user namespaces, an unsupported Bubblewrap option, or failed checks return
/// errors. This never retries with fewer restrictions, elevates privileges, or launches user input.
pub fn probe_linux_bubblewrap() -> Result<SandboxProbeEvidence, SandboxProbeError> {
    #[cfg(target_os = "linux")]
    {
        linux::probe()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(SandboxProbeError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{SandboxProbeError, SandboxProbeEvidence};
    use crate::selected_snapshots::{SelectedSnapshotBundle, SelectedSnapshotPolicy};
    use carapana_storage::WorkspaceMetadata;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        path::{Path, PathBuf},
        process::{Command, ExitStatus, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const BACKEND: &str = "/usr/bin/bwrap";
    const OUTPUT_LIMIT: usize = 4096;
    const DEADLINE: Duration = Duration::from_secs(5);
    const SUCCESS: &[u8] = b"carapana-bubblewrap-probe-v1\n";
    const SCRIPT: &str = r#"
import errno, os, socket, sys
assert set(os.environ) == {'PATH', 'LC_ALL', 'PWD'}
assert os.environ.get('PATH') == '/usr/bin:/bin'
assert os.environ['LC_ALL'] == 'C'
assert os.environ['PWD'] == '/workspace'
for kind, host in zip(('user', 'pid', 'net'), sys.argv[1:4]):
    assert os.readlink('/proc/self/ns/' + kind) != host
status = dict(line.split(':', 1) for line in open('/proc/self/status') if ':' in line)
assert status['NoNewPrivs'].strip() == '1'
assert int(status['CapEff'].strip(), 16) == 0
assert open('/workspace/input', 'rb').read() == b'synthetic input\n'
assert not os.path.exists('/workspace/.env.example')
assert not os.path.exists(sys.argv[4])
assert not os.path.exists('/etc/passwd')
assert not os.path.exists('/home')
try:
    open('/workspace/modified', 'wb')
except OSError as error:
    assert error.errno in (errno.EROFS, errno.EACCES, errno.EPERM)
else:
    raise AssertionError('workspace was writable')
try:
    open('/root-write', 'wb')
except OSError as error:
    assert error.errno in (errno.EROFS, errno.EACCES, errno.EPERM)
else:
    raise AssertionError('sandbox root was writable')
with open('/tmp/probe', 'wb') as output:
    output.write(b'synthetic scratch\n')
client = socket.socket()
client.settimeout(0.25)
try:
    client.connect(('127.0.0.1', int(sys.argv[5])))
except OSError as error:
    assert error.errno in (errno.ECONNREFUSED, errno.ENETUNREACH, errno.EHOSTUNREACH, errno.EACCES)
else:
    raise AssertionError('host loopback was reachable')
finally:
    client.close()
print('carapana-bubblewrap-probe-v1')
"#;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Result<Self, SandboxProbeError> {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let root = fs::canonicalize(std::env::temp_dir())
                .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| SandboxProbeError::FixtureUnavailable)?
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "carapana-sandbox-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
            let fixture = Self(path);
            fs::DirBuilder::new()
                .mode(0o700)
                .create(fixture.0.join("workspace"))
                .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
            for (name, contents) in [
                ("workspace/input", b"synthetic input\n".as_slice()),
                (
                    "workspace/.env.example",
                    b"synthetic protected fixture\n".as_slice(),
                ),
                ("outside-canary", b"synthetic outside\n".as_slice()),
            ] {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(fixture.0.join(name))
                    .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
                file.write_all(contents)
                    .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
            }
            Ok(fixture)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn validate_backend(path: &Path) -> Result<(), SandboxProbeError> {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| SandboxProbeError::BackendUnavailable)?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o6022 != 0
            || metadata.mode() & 0o111 == 0
        {
            return Err(SandboxProbeError::UnsafeBackend);
        }
        Ok(())
    }

    fn read_bounded(reader: impl Read) -> Result<Vec<u8>, SandboxProbeError> {
        let mut bytes = Vec::new();
        reader
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SandboxProbeError::EnforcementNotVerified)?;
        if bytes.len() > OUTPUT_LIMIT {
            return Err(SandboxProbeError::OutputLimitExceeded);
        }
        Ok(bytes)
    }

    pub(super) fn probe() -> Result<SandboxProbeEvidence, SandboxProbeError> {
        validate_backend(Path::new(BACKEND))?;
        let fixture = Fixture::new()?;
        let source = WorkspaceMetadata::capture(fixture.0.join("workspace"))
            .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
        let policy = SelectedSnapshotPolicy::new(1, 1024)
            .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
        let selected = SelectedSnapshotBundle::prepare(&source, &[PathBuf::from("input")], policy)
            .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
        let port = listener
            .local_addr()
            .map_err(|_| SandboxProbeError::FixtureUnavailable)?
            .port();
        let mut command = Command::new(BACKEND);
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.args([
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--setenv",
            "LC_ALL",
            "C",
        ]);
        for path in ["/usr", "/bin", "/lib", "/lib64"] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }
        command
            .arg("--ro-bind")
            .arg(selected.staging_root())
            .arg("/workspace");
        command.args([
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--size",
            "1048576",
            "--tmpfs",
            "/tmp",
            "--chdir",
            "/workspace",
            "--remount-ro",
            "/",
            "--",
            "/usr/bin/python3",
            "-I",
            "-c",
            SCRIPT,
        ]);
        for kind in ["user", "pid", "net"] {
            let identity = fs::read_link(format!("/proc/self/ns/{kind}"))
                .map_err(|_| SandboxProbeError::FixtureUnavailable)?;
            command.arg(identity);
        }
        command
            .arg(fixture.0.join("outside-canary"))
            .arg(port.to_string());
        let (status, stdout, stderr) = run_probe_command(command, DEADLINE)?;
        if !status.success() || stdout != SUCCESS || !stderr.is_empty() {
            return Err(SandboxProbeError::EnforcementNotVerified);
        }
        selected
            .verify()
            .map_err(|_| SandboxProbeError::EnforcementNotVerified)?;
        if fixture.0.join("workspace/modified").exists()
            || fs::read(fixture.0.join("workspace/input")).ok().as_deref()
                != Some(b"synthetic input\n")
        {
            return Err(SandboxProbeError::EnforcementNotVerified);
        }
        Ok(SandboxProbeEvidence { _private: () })
    }

    fn run_probe_command(
        mut command: Command,
        deadline: Duration,
    ) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), SandboxProbeError> {
        let mut child = command
            .spawn()
            .map_err(|_| SandboxProbeError::SpawnFailed)?;
        let stdout = child.stdout.take().ok_or(SandboxProbeError::SpawnFailed)?;
        let stderr = child.stderr.take().ok_or(SandboxProbeError::SpawnFailed)?;
        let stdout = thread::spawn(move || read_bounded(stdout));
        let stderr = thread::spawn(move || read_bounded(stderr));
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if start.elapsed() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(SandboxProbeError::TimedOut);
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(SandboxProbeError::EnforcementNotVerified);
                }
            }
        };
        let stdout = stdout
            .join()
            .map_err(|_| SandboxProbeError::EnforcementNotVerified)?;
        let stderr = stderr
            .join()
            .map_err(|_| SandboxProbeError::EnforcementNotVerified)?;
        let status = status?;
        let stdout = stdout?;
        let stderr = stderr?;
        Ok((status, stdout, stderr))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn should_bound_probe_output_before_allocating_unlimited_bytes() {
            assert_eq!(read_bounded(&b"small"[..]).unwrap(), b"small");
            assert_eq!(
                read_bounded(std::io::repeat(0)),
                Err(SandboxProbeError::OutputLimitExceeded)
            );
        }

        #[test]
        fn should_enforce_deadline_and_reap_the_probe_child() {
            let mut command = Command::new("/usr/bin/python3");
            command
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command.args(["-I", "-c", "import time; time.sleep(30)"]);
            let started = Instant::now();
            assert_eq!(
                run_probe_command(command, Duration::from_millis(50)).unwrap_err(),
                SandboxProbeError::TimedOut
            );
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[test]
        fn should_bound_both_real_output_pipes() {
            for descriptor in ["1", "2"] {
                let mut command = Command::new("/usr/bin/python3");
                command
                    .env_clear()
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                command.args([
                    "-I",
                    "-c",
                    &format!("import os; os.write({descriptor}, b'x' * 8192)"),
                ]);
                assert_eq!(
                    run_probe_command(command, DEADLINE).unwrap_err(),
                    SandboxProbeError::OutputLimitExceeded
                );
            }
        }

        #[test]
        fn should_reject_missing_and_non_system_backends() {
            let fixture = Fixture::new().unwrap();
            assert_eq!(
                validate_backend(&fixture.0.join("missing")),
                Err(SandboxProbeError::BackendUnavailable)
            );
            assert_eq!(
                validate_backend(&fixture.0.join("workspace/input")),
                Err(SandboxProbeError::UnsafeBackend)
            );
        }

        #[test]
        fn should_probe_real_bubblewrap_or_report_unavailable_without_fallback() {
            let result = super::probe();
            if std::env::var_os("CARAPANA_REQUIRE_BUBBLEWRAP").is_some() {
                assert!(
                    result.is_ok(),
                    "required sandbox enforcement was not verified: {result:?}"
                );
            } else {
                assert!(matches!(
                    result,
                    Ok(_)
                        | Err(SandboxProbeError::BackendUnavailable
                            | SandboxProbeError::EnforcementNotVerified)
                ));
            }
        }
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn should_fail_closed_on_unsupported_platforms() {
        assert_eq!(
            probe_linux_bubblewrap(),
            Err(SandboxProbeError::UnsupportedPlatform)
        );
    }
}
