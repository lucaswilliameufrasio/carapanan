//! Capture within the existing private process guardian. Never exposed as a model tool or daemon IPC.

use super::*;
use crate::process_output::{
    OutputCapturePolicy, OutputStream,
    pipes::{OutputDrainOutcome, ProcessOutputPipes},
};

const MAX_TRANSPORT_TAIL_BYTES: usize = 8192;
const MAX_CAPTURE_TIMEOUT_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCaptureSettings {
    stdout_tail_bytes: usize,
    stderr_tail_bytes: usize,
    max_capture_bytes: u64,
    timeout_ms: u64,
}

impl ProcessCaptureSettings {
    pub fn new(
        stdout_tail_bytes: usize,
        stderr_tail_bytes: usize,
        max_capture_bytes: u64,
        timeout: Duration,
    ) -> Result<Self, ProcessError> {
        let timeout_ms =
            u64::try_from(timeout.as_millis()).map_err(|_| ProcessError::InvalidProcessSpec)?;
        let settings = Self {
            stdout_tail_bytes,
            stderr_tail_bytes,
            max_capture_bytes,
            timeout_ms,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub(super) fn validate(self) -> Result<OutputCapturePolicy, ProcessError> {
        if self.timeout_ms == 0
            || self.timeout_ms > MAX_CAPTURE_TIMEOUT_MS
            || self
                .stdout_tail_bytes
                .checked_add(self.stderr_tail_bytes)
                .is_none_or(|total| total > MAX_TRANSPORT_TAIL_BYTES)
        {
            return Err(ProcessError::InvalidProcessSpec);
        }
        OutputCapturePolicy::new(
            self.stdout_tail_bytes,
            self.stderr_tail_bytes,
            self.max_capture_bytes,
        )
        .map_err(|_| ProcessError::InvalidProcessSpec)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturedProcessEnd {
    Exited,
    CaptureLimit,
    Cancelled,
    TimedOut,
}

/// Raw untrusted tails. No Debug, Clone, serialization, automatic persistence or provider forwarding.
///
/// ```compile_fail
/// fn log(output: &carapana_daemon::OwnedProcessOutput) {
///     let _unsafe_log = format!("{output:?}");
/// }
/// ```
pub struct OwnedProcessOutput {
    end: CapturedProcessEnd,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_bytes: u64,
    stderr_bytes: u64,
}

impl OwnedProcessOutput {
    pub fn end(&self) -> CapturedProcessEnd {
        self.end
    }
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    /// Untrusted bytes, not safe to log/display/share without independent checks.
    pub fn stdout_tail(&self) -> &[u8] {
        &self.stdout
    }
    /// Untrusted bytes, not safe to log/display/share without independent checks.
    pub fn stderr_tail(&self) -> &[u8] {
        &self.stderr
    }
    pub fn stdout_bytes(&self) -> u64 {
        self.stdout_bytes
    }
    pub fn stderr_bytes(&self) -> u64 {
        self.stderr_bytes
    }
}

// Only this private wire DTO is serialized, over the private guardian pipe with a 64 KiB frame cap.
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum HostCapturedReport {
    Complete {
        end: CapturedProcessEnd,
        exit_code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        stdout_bytes: u64,
        stderr_bytes: u64,
    },
    Failed,
}

impl HostCapturedReport {
    pub(super) fn validate(
        self,
        settings: ProcessCaptureSettings,
    ) -> Result<OwnedProcessOutput, ProcessError> {
        settings.validate()?;
        let Self::Complete {
            end,
            exit_code,
            stdout,
            stderr,
            stdout_bytes,
            stderr_bytes,
        } = self
        else {
            return Err(ProcessError::HostProtocol);
        };
        let total = stdout_bytes
            .checked_add(stderr_bytes)
            .ok_or(ProcessError::HostProtocol)?;
        if stdout.len() > settings.stdout_tail_bytes
            || stderr.len() > settings.stderr_tail_bytes
            || stdout_bytes < stdout.len() as u64
            || stderr_bytes < stderr.len() as u64
            || total > settings.max_capture_bytes
            || stdout.len() as u64 != stdout_bytes.min(settings.stdout_tail_bytes as u64)
            || stderr.len() as u64 != stderr_bytes.min(settings.stderr_tail_bytes as u64)
            || (end == CapturedProcessEnd::CaptureLimit && total != settings.max_capture_bytes)
            || (end != CapturedProcessEnd::Exited && exit_code.is_some())
        {
            return Err(ProcessError::HostProtocol);
        }
        Ok(OwnedProcessOutput {
            end,
            exit_code,
            stdout,
            stderr,
            stdout_bytes,
            stderr_bytes,
        })
    }
}

impl ProcessSupervisor {
    /// Synchronously run one explicitly specified owned process with finite capture and a deadline.
    /// Internal primitive only: no sandbox/authorization claims, no daemon IPC, no model tool wiring.
    /// `KeepRunning` is rejected. Returns only after the guardian exited and finished group cleanup.
    pub fn run_captured_owned_process(
        &mut self,
        session_id: &str,
        mut spec: ProcessSpec,
        settings: ProcessCaptureSettings,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<OwnedProcessOutput, ProcessError> {
        settings.validate()?;
        if spec.shutdown_policy != ProcessShutdownPolicy::Terminate {
            return Err(ProcessError::InvalidProcessSpec);
        }
        // Cancellation before admission must not start a producer or consume a process slot.
        if is_cancelled() {
            return Ok(OwnedProcessOutput {
                end: CapturedProcessEnd::Cancelled,
                exit_code: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdout_bytes: 0,
                stderr_bytes: 0,
            });
        }
        spec.capture = Some(settings);
        let info = self.spawn_owned_process(session_id, spec)?;
        let mut process = self
            .processes
            .remove(&info.id)
            .ok_or(ProcessError::ProcessNotFound)?;
        // This method holds &mut self for the entire run, so no other launch can take its temporarily
        // removed slot. On failure, closing the controller transfers cleanup to the guardian.
        let receiver = process.output.take().ok_or(ProcessError::HostProtocol)?;
        let deadline =
            Instant::now() + Duration::from_millis(settings.timeout_ms) + HOST_SHUTDOWN_TIMEOUT;
        let mut stop_requested = false;
        let result = loop {
            if !stop_requested && is_cancelled() {
                if write_frame(&mut process.control, &HostControl::Stop).is_err() {
                    break Err(ProcessError::HostProtocol);
                }
                stop_requested = true;
            }
            match receiver.recv_timeout(POLL_INTERVAL) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(ProcessError::HostProtocol),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    break Err(ProcessError::HostTimedOut);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        drop(process.control);
        wait_for_host_exit(&mut process.host, HOST_SHUTDOWN_TIMEOUT)?;
        result
    }
}

pub(super) fn run_host(
    request: HostStartRequest,
    terminate_on_signal: Arc<AtomicBool>,
) -> Result<(), ProcessError> {
    let settings = request.capture.ok_or(ProcessError::HostRejected)?;
    let policy = settings.validate()?;
    if request.shutdown_policy != ProcessShutdownPolicy::Terminate {
        return Err(ProcessError::HostRejected);
    }
    let mut child = match spawn_process(&request) {
        Ok(child) => child,
        Err(_) => {
            write_frame(&mut io::stdout().lock(), &HostStartResponse::Rejected)?;
            return Ok(());
        }
    };
    let started = write_frame(
        &mut io::stdout().lock(),
        &HostStartResponse::Started {
            process_id: child.id(),
            process_group_id: std::process::id(),
        },
    );
    if started.is_err() {
        terminate_own_process_group()?;
        return Err(ProcessError::HostProtocol);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let control = read_frame::<_, HostControl>(&mut io::stdin().lock()).ok();
        let _ = sender.send(control);
    });
    let mut should_cancel = || {
        terminate_on_signal.load(Ordering::Acquire)
            || !matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty))
    };
    let deadline = Instant::now() + Duration::from_millis(settings.timeout_ms);
    let report = collect(&mut child, policy, deadline, &mut should_cancel)
        .unwrap_or(HostCapturedReport::Failed);
    let sent = write_frame(&mut io::stdout().lock(), &report);
    // This guardian is the group leader; report transport never relinquishes group ownership.
    terminate_own_process_group()?;
    sent
}

fn collect(
    child: &mut Child,
    policy: OutputCapturePolicy,
    deadline: Instant,
    should_cancel: &mut impl FnMut() -> bool,
) -> Result<HostCapturedReport, ProcessError> {
    let stdout = child.stdout.take().ok_or(ProcessError::HostProtocol)?;
    let stderr = child.stderr.take().ok_or(ProcessError::HostProtocol)?;
    let mut pipes =
        ProcessOutputPipes::new(stdout, stderr, policy).map_err(|_| ProcessError::HostProtocol)?;
    let outcome = pipes
        .drain_until(deadline, &mut *should_cancel)
        .map_err(|_| ProcessError::HostProtocol)?;
    let mut exit_code = None;
    let end = match outcome {
        OutputDrainOutcome::CaptureLimit => CapturedProcessEnd::CaptureLimit,
        OutputDrainOutcome::Cancelled => CapturedProcessEnd::Cancelled,
        OutputDrainOutcome::TimedOut => CapturedProcessEnd::TimedOut,
        OutputDrainOutcome::EndOfStreams => loop {
            if should_cancel() {
                break CapturedProcessEnd::Cancelled;
            }
            if Instant::now() >= deadline {
                break CapturedProcessEnd::TimedOut;
            }
            if let Some(status) = child.try_wait()? {
                exit_code = status.code();
                break CapturedProcessEnd::Exited;
            }
            thread::sleep(POLL_INTERVAL);
        },
    };
    let capture = pipes.capture();
    let tail = |stream| {
        let (first, second) = capture.tail_slices(stream);
        let mut bytes = Vec::with_capacity(first.len() + second.len());
        bytes.extend_from_slice(first);
        bytes.extend_from_slice(second);
        bytes
    };
    Ok(HostCapturedReport::Complete {
        end,
        exit_code,
        stdout: tail(OutputStream::Stdout),
        stderr: tail(OutputStream::Stderr),
        stdout_bytes: capture.stats(OutputStream::Stdout).accepted_bytes,
        stderr_bytes: capture.stats(OutputStream::Stderr).accepted_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_validate_capture_and_private_transport_budgets() {
        for (out, err, total, timeout) in [
            (8192, 1, 16384, 1000),
            (1, 1, 1, 1000),
            (1, 1, 64, 0),
            (1, 1, 64, MAX_CAPTURE_TIMEOUT_MS + 1),
        ] {
            assert!(
                ProcessCaptureSettings::new(out, err, total, Duration::from_millis(timeout))
                    .is_err()
            );
        }
        let settings =
            ProcessCaptureSettings::new(4096, 4096, 8192, Duration::from_secs(1)).unwrap();
        let report = HostCapturedReport::Complete {
            end: CapturedProcessEnd::CaptureLimit,
            exit_code: None,
            stdout: vec![255; 4096],
            stderr: vec![255; 4096],
            stdout_bytes: 4096,
            stderr_bytes: 4096,
        };
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &report).unwrap();
        assert!(encoded.len() <= MAX_FRAME_BYTES + 4);
        assert!(report.validate(settings).is_ok());
    }

    #[test]
    fn should_reject_inconsistent_output_reports() {
        let settings = ProcessCaptureSettings::new(2, 2, 8, Duration::from_secs(1)).unwrap();
        let report = HostCapturedReport::Complete {
            end: CapturedProcessEnd::Cancelled,
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
            stdout_bytes: 0,
            stderr_bytes: 0,
        };
        assert!(matches!(
            report.validate(settings),
            Err(ProcessError::HostProtocol)
        ));
        let report = HostCapturedReport::Complete {
            end: CapturedProcessEnd::Exited,
            exit_code: Some(0),
            stdout: vec![1; 3],
            stderr: vec![],
            stdout_bytes: 3,
            stderr_bytes: 0,
        };
        assert!(matches!(
            report.validate(settings),
            Err(ProcessError::HostProtocol)
        ));
    }

    #[test]
    fn should_reject_missing_tail_bytes_and_false_capture_limit_reports() {
        let settings = ProcessCaptureSettings::new(2, 2, 8, Duration::from_secs(1)).unwrap();
        for report in [
            HostCapturedReport::Complete {
                end: CapturedProcessEnd::Exited,
                exit_code: Some(0),
                stdout: vec![],
                stderr: vec![],
                stdout_bytes: 3,
                stderr_bytes: 0,
            },
            HostCapturedReport::Complete {
                end: CapturedProcessEnd::CaptureLimit,
                exit_code: None,
                stdout: vec![1; 2],
                stderr: vec![],
                stdout_bytes: 3,
                stderr_bytes: 0,
            },
            HostCapturedReport::Complete {
                end: CapturedProcessEnd::Exited,
                exit_code: None,
                stdout: vec![1; 2],
                stderr: vec![1; 2],
                stdout_bytes: u64::MAX,
                stderr_bytes: 3,
            },
        ] {
            assert!(matches!(
                report.validate(settings),
                Err(ProcessError::HostProtocol)
            ));
        }
    }
}
