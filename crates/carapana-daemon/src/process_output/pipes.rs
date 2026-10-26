//! Bounded nonblocking readers for already-owned child pipes. No process spawning or termination.

use super::{OutputCapturePolicy, OutputStream, ProcessOutputCapture};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use std::{
    io::{self, Read},
    process::{ChildStderr, ChildStdout},
    thread,
    time::{Duration, Instant},
};

const READ_CHUNK_BYTES: usize = 4096;
const IDLE_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPipeError {
    NonblockingUnavailable,
    ReadFailed,
    CaptureFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputDrainOutcome {
    /// Both pipes reached EOF. Descendants/processes may still be alive with stdio closed.
    EndOfStreams,
    CaptureLimit,
    Cancelled,
    TimedOut,
}

/// Owns both pipes and private volatile tails; deliberately not Debug, Clone or serializable.
/// A caller must still own and supervise the producer. Dropping this value only closes the pipes.
///
/// ```compile_fail
/// fn log(output: &carapana_daemon::process_output::pipes::ProcessOutputPipes) {
///     let _unsafe_log = format!("{output:?}");
/// }
/// ```
pub struct ProcessOutputPipes {
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    capture: ProcessOutputCapture,
    stderr_first: bool,
}

impl ProcessOutputPipes {
    pub fn new(
        stdout: ChildStdout,
        stderr: ChildStderr,
        policy: OutputCapturePolicy,
    ) -> Result<Self, OutputPipeError> {
        fcntl_setfl(
            &stdout,
            fcntl_getfl(&stdout).map_err(|_| OutputPipeError::NonblockingUnavailable)?
                | OFlags::NONBLOCK,
        )
        .map_err(|_| OutputPipeError::NonblockingUnavailable)?;
        fcntl_setfl(
            &stderr,
            fcntl_getfl(&stderr).map_err(|_| OutputPipeError::NonblockingUnavailable)?
                | OFlags::NONBLOCK,
        )
        .map_err(|_| OutputPipeError::NonblockingUnavailable)?;
        Ok(Self {
            stdout: Some(stdout),
            stderr: Some(stderr),
            capture: ProcessOutputCapture::new(policy),
            stderr_first: false,
        })
    }

    pub fn capture(&self) -> &ProcessOutputCapture {
        &self.capture
    }

    /// Drain without waiting indefinitely for an idle stream or descendants holding a pipe open.
    /// The monotonic deadline is caller supplied. On all stop/error results the caller must supervise
    /// termination/reaping separately; the producer is never killed or detached by this method.
    pub fn drain_until(
        &mut self,
        deadline: Instant,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<OutputDrainOutcome, OutputPipeError> {
        loop {
            if is_cancelled() {
                return Ok(OutputDrainOutcome::Cancelled);
            }
            if Instant::now() >= deadline {
                return Ok(OutputDrainOutcome::TimedOut);
            }
            if self.capture.budget_exhausted() {
                return Ok(OutputDrainOutcome::CaptureLimit);
            }
            if self.stdout.is_none() && self.stderr.is_none() {
                return Ok(OutputDrainOutcome::EndOfStreams);
            }
            let streams = if self.stderr_first {
                [OutputStream::Stderr, OutputStream::Stdout]
            } else {
                [OutputStream::Stdout, OutputStream::Stderr]
            };
            self.stderr_first = !self.stderr_first;
            let mut progressed = false;
            for stream in streams {
                if is_cancelled() {
                    return Ok(OutputDrainOutcome::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Ok(OutputDrainOutcome::TimedOut);
                }
                if self.capture.budget_exhausted() {
                    return Ok(OutputDrainOutcome::CaptureLimit);
                }
                progressed |= self.read_once(stream)?;
            }
            if !progressed {
                thread::sleep(
                    IDLE_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }

    fn read_once(&mut self, stream: OutputStream) -> Result<bool, OutputPipeError> {
        let remaining = self.capture.policy.max_capture_bytes - self.capture.accepted_bytes();
        let requested = READ_CHUNK_BYTES.min(remaining as usize);
        let mut chunk = [0_u8; READ_CHUNK_BYTES];
        let count = match stream {
            OutputStream::Stdout => match self.stdout.as_mut() {
                Some(pipe) => read_nonblocking(pipe, &mut chunk[..requested])?,
                None => return Ok(false),
            },
            OutputStream::Stderr => match self.stderr.as_mut() {
                Some(pipe) => read_nonblocking(pipe, &mut chunk[..requested])?,
                None => return Ok(false),
            },
        };
        let Some(count) = count else {
            return Ok(false);
        };
        if count == 0 {
            match stream {
                OutputStream::Stdout => self.stdout = None,
                OutputStream::Stderr => self.stderr = None,
            }
            return Ok(true);
        }
        self.capture
            .append(stream, &chunk[..count])
            .map_err(|_| OutputPipeError::CaptureFailed)?;
        Ok(true)
    }
}

fn read_nonblocking(
    reader: &mut impl Read,
    chunk: &mut [u8],
) -> Result<Option<usize>, OutputPipeError> {
    match reader.read(chunk) {
        Ok(count) => Ok(Some(count)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(_) => Err(OutputPipeError::ReadFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    struct Producer(Child);
    impl Producer {
        fn spawn(script: &str) -> Self {
            Self(
                Command::new("/usr/bin/python3")
                    .args(["-I", "-c", script])
                    .env_clear()
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            )
        }
        fn pipes(&mut self, policy: OutputCapturePolicy) -> ProcessOutputPipes {
            ProcessOutputPipes::new(
                self.0.stdout.take().unwrap(),
                self.0.stderr.take().unwrap(),
                policy,
            )
            .unwrap()
        }
    }
    impl Drop for Producer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn bytes(pipes: &ProcessOutputPipes, stream: OutputStream) -> Vec<u8> {
        let (first, second) = pipes.capture().tail_slices(stream);
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(second);
        bytes
    }

    #[test]
    fn should_drain_both_real_pipes_without_losing_binary_bytes() {
        let mut producer =
            Producer::spawn("import os; os.write(1, b'out\\x00\\xff'); os.write(2, b'err\\x1b')");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(32, 32, 128).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::EndOfStreams)
        );
        assert_eq!(bytes(&pipes, OutputStream::Stdout), b"out\x00\xff");
        assert_eq!(bytes(&pipes, OutputStream::Stderr), b"err\x1b");
        assert!(producer.0.wait().unwrap().success());
    }

    #[test]
    fn should_drain_output_larger_than_pipe_capacity_without_a_reader_queue() {
        let mut producer = Producer::spawn(
            "import os; chunk=b'x'*4096\nfor _ in range(64):\n os.write(1,chunk); os.write(2,chunk)",
        );
        let mut pipes = producer.pipes(OutputCapturePolicy::new(17, 19, 1024 * 1024).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::EndOfStreams)
        );
        assert_eq!(pipes.capture().accepted_bytes(), 512 * 1024);
        assert_eq!(bytes(&pipes, OutputStream::Stdout), vec![b'x'; 17]);
        assert_eq!(bytes(&pipes, OutputStream::Stderr), vec![b'x'; 19]);
        assert_eq!(pipes.capture().retained_bytes(), 36);
        assert!(producer.0.wait().unwrap().success());
    }

    #[test]
    fn should_stop_at_the_exact_shared_capture_limit_during_a_flood() {
        let mut producer = Producer::spawn("import os\nwhile True: os.write(1,b'x'*4096)");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(8, 8, 8193).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::CaptureLimit)
        );
        assert_eq!(pipes.capture().accepted_bytes(), 8193);
        assert!(pipes.capture().retained_bytes() <= 16);
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::CaptureLimit)
        );
        assert_eq!(pipes.capture().accepted_bytes(), 8193);
    }

    #[test]
    fn should_honor_timeout_with_one_pipe_closed_and_the_other_silent() {
        let mut producer = Producer::spawn("import os,time; os.close(1); time.sleep(30)");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(8, 8, 64).unwrap());
        let start = Instant::now();
        assert_eq!(
            pipes.drain_until(start + Duration::from_millis(100), || false),
            Ok(OutputDrainOutcome::TimedOut)
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(pipes.capture().accepted_bytes(), 0);
    }

    #[test]
    fn should_check_cancellation_before_reading_and_between_chunks() {
        let mut producer = Producer::spawn("import os,time; os.write(1,b'x'*4096); time.sleep(30)");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(8, 8, 65536).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || true),
            Ok(OutputDrainOutcome::Cancelled)
        );
        assert_eq!(pipes.capture().accepted_bytes(), 0);
        assert_eq!(
            pipes.drain_until(Instant::now(), || false),
            Ok(OutputDrainOutcome::TimedOut)
        );
        let mut checks = 0;
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || {
                checks += 1;
                checks > 10
            }),
            Ok(OutputDrainOutcome::Cancelled)
        );
        assert!(checks > 10);
    }

    #[test]
    fn should_not_treat_eof_as_proof_that_the_producer_exited() {
        let mut producer =
            Producer::spawn("import os,time; os.close(1); os.close(2); time.sleep(30)");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(8, 8, 64).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::EndOfStreams)
        );
        assert!(producer.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn should_continue_reading_stderr_while_stdout_is_silent() {
        let mut producer =
            Producer::spawn("import os,time; os.write(2,b'error'*1000); time.sleep(30)");
        let mut pipes = producer.pipes(OutputCapturePolicy::new(8, 15, 5000).unwrap());
        assert_eq!(
            pipes.drain_until(Instant::now() + Duration::from_secs(5), || false),
            Ok(OutputDrainOutcome::CaptureLimit)
        );
        assert_eq!(
            pipes.capture().stats(OutputStream::Stdout).accepted_bytes,
            0
        );
        assert_eq!(
            pipes.capture().stats(OutputStream::Stderr).accepted_bytes,
            5000
        );
        assert_eq!(bytes(&pipes, OutputStream::Stderr), b"errorerrorerror");
    }

    #[test]
    fn should_return_at_deadline_when_a_descendant_keeps_the_pipes_open() {
        use std::os::unix::process::CommandExt;
        struct OwnedFixtureGroup(Child);
        impl Drop for OwnedFixtureGroup {
            fn drop(&mut self) {
                // The direct child is never reaped before this destructor, so its PID/PGID cannot
                // be reused while killing only this test-created group (including its grandchild).
                let id = nix::unistd::Pid::from_raw(i32::try_from(self.0.id()).unwrap());
                let _ = nix::sys::signal::killpg(id, nix::sys::signal::Signal::SIGKILL);
                let _ = self.0.wait();
            }
        }
        let script = "import os,subprocess\np=subprocess.Popen(['/usr/bin/python3','-I','-c','import time; time.sleep(30)'])\nos.close(1); os.close(2); p.wait()";
        let child = Command::new("/usr/bin/python3")
            .args(["-I", "-c", script])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut producer = OwnedFixtureGroup(child);
        let mut pipes = ProcessOutputPipes::new(
            producer.0.stdout.take().unwrap(),
            producer.0.stderr.take().unwrap(),
            OutputCapturePolicy::new(8, 8, 64).unwrap(),
        )
        .unwrap();
        let start = Instant::now();
        assert_eq!(
            pipes.drain_until(start + Duration::from_millis(200), || false),
            Ok(OutputDrainOutcome::TimedOut)
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
