//! Bounded, volatile output tails. No subprocesses, I/O, rendering, persistence, or secret classification.
//!
//! Callers must bound each read before allocating a chunk, stop at the shared capture limit, and use
//! the process supervisor to cancel producers. A saturated buffer does not stop a process or drain pipes.

use std::collections::VecDeque;

pub const MAX_OUTPUT_TAIL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OUTPUT_CAPTURE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputCaptureError {
    InvalidPolicy,
    AllocationFailed,
}

/// Explicit payload budgets, not a total-RSS or process resource policy. There are no default limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputCapturePolicy {
    stdout_tail_bytes: usize,
    stderr_tail_bytes: usize,
    max_capture_bytes: u64,
}

impl OutputCapturePolicy {
    pub fn new(
        stdout_tail_bytes: usize,
        stderr_tail_bytes: usize,
        max_capture_bytes: u64,
    ) -> Result<Self, OutputCaptureError> {
        let tail_bytes = stdout_tail_bytes
            .checked_add(stderr_tail_bytes)
            .ok_or(OutputCaptureError::InvalidPolicy)?;
        if tail_bytes > MAX_OUTPUT_TAIL_BYTES
            || max_capture_bytes == 0
            || max_capture_bytes > MAX_OUTPUT_CAPTURE_BYTES
            || tail_bytes as u64 > max_capture_bytes
        {
            return Err(OutputCaptureError::InvalidPolicy);
        }
        Ok(Self {
            stdout_tail_bytes,
            stderr_tail_bytes,
            max_capture_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturedChunk {
    pub accepted_bytes: usize,
    pub rejected_bytes: usize,
    /// The shared limit is reached: callers must not infer EOF or process completion from this.
    pub budget_exhausted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCaptureStats {
    pub accepted_bytes: u64,
    pub retained_bytes: usize,
    pub discarded_tail_bytes: u64,
}

/// Raw output intentionally does not implement `Debug`, `Clone`, or serialization.
///
/// ```compile_fail
/// use carapana_daemon::process_output::{OutputCapturePolicy, ProcessOutputCapture};
/// let capture = ProcessOutputCapture::new(OutputCapturePolicy::new(4, 4, 8).unwrap());
/// let _unsafe_log = format!("{capture:?}");
/// ```
pub struct ProcessOutputCapture {
    policy: OutputCapturePolicy,
    stdout: StreamTail,
    stderr: StreamTail,
    accepted_bytes: u64,
}

impl ProcessOutputCapture {
    pub fn new(policy: OutputCapturePolicy) -> Self {
        Self {
            policy,
            stdout: StreamTail::new(),
            stderr: StreamTail::new(),
            accepted_bytes: 0,
        }
    }

    /// Accept only the prefix fitting the shared budget, retaining that stream's newest tail bytes.
    /// This method performs no reads; passing an already-unbounded chunk cannot bound its allocation.
    pub fn append(
        &mut self,
        stream: OutputStream,
        chunk: &[u8],
    ) -> Result<CapturedChunk, OutputCaptureError> {
        let remaining = self
            .policy
            .max_capture_bytes
            .saturating_sub(self.accepted_bytes);
        let accepted = chunk.len().min(remaining as usize);
        let (tail, tail_limit) = match stream {
            OutputStream::Stdout => (&mut self.stdout, self.policy.stdout_tail_bytes),
            OutputStream::Stderr => (&mut self.stderr, self.policy.stderr_tail_bytes),
        };
        tail.append(&chunk[..accepted], tail_limit)?;
        self.accepted_bytes += accepted as u64;
        Ok(CapturedChunk {
            accepted_bytes: accepted,
            rejected_bytes: chunk.len() - accepted,
            budget_exhausted: self.budget_exhausted(),
        })
    }

    pub fn accepted_bytes(&self) -> u64 {
        self.accepted_bytes
    }
    pub fn budget_exhausted(&self) -> bool {
        self.accepted_bytes == self.policy.max_capture_bytes
    }
    pub fn retained_bytes(&self) -> usize {
        self.stdout.bytes.len() + self.stderr.bytes.len()
    }

    /// At most two slices of raw untrusted bytes, in stream order. Not safe to display/log/share.
    pub fn tail_slices(&self, stream: OutputStream) -> (&[u8], &[u8]) {
        self.tail(stream).bytes.as_slices()
    }

    pub fn stats(&self, stream: OutputStream) -> StreamCaptureStats {
        let tail = self.tail(stream);
        StreamCaptureStats {
            accepted_bytes: tail.accepted_bytes,
            retained_bytes: tail.bytes.len(),
            discarded_tail_bytes: tail.accepted_bytes - tail.bytes.len() as u64,
        }
    }

    fn tail(&self, stream: OutputStream) -> &StreamTail {
        match stream {
            OutputStream::Stdout => &self.stdout,
            OutputStream::Stderr => &self.stderr,
        }
    }
}

struct StreamTail {
    bytes: VecDeque<u8>,
    accepted_bytes: u64,
}

impl StreamTail {
    fn new() -> Self {
        Self {
            bytes: VecDeque::new(),
            accepted_bytes: 0,
        }
    }

    fn append(&mut self, chunk: &[u8], limit: usize) -> Result<(), OutputCaptureError> {
        if !chunk.is_empty() && limit > 0 {
            if self.bytes.capacity() < limit {
                self.bytes
                    .try_reserve_exact(limit - self.bytes.len())
                    .map_err(|_| OutputCaptureError::AllocationFailed)?;
            }
            if chunk.len() >= limit {
                self.bytes.clear();
                self.bytes.extend(&chunk[chunk.len() - limit..]);
            } else {
                let evict = (self.bytes.len() + chunk.len()).saturating_sub(limit);
                for _ in 0..evict {
                    self.bytes.pop_front();
                }
                self.bytes.extend(chunk);
            }
        }
        self.accepted_bytes += chunk.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(capture: &ProcessOutputCapture, stream: OutputStream) -> Vec<u8> {
        let (first, second) = capture.tail_slices(stream);
        let mut result = first.to_vec();
        result.extend_from_slice(second);
        result
    }

    #[test]
    fn should_share_one_capture_budget_across_stdout_and_stderr() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(4, 4, 8).unwrap());
        assert_eq!(
            capture
                .append(OutputStream::Stdout, b"12345")
                .unwrap()
                .accepted_bytes,
            5
        );
        let result = capture.append(OutputStream::Stderr, b"abcde").unwrap();
        assert_eq!(
            result,
            CapturedChunk {
                accepted_bytes: 3,
                rejected_bytes: 2,
                budget_exhausted: true
            }
        );
        assert_eq!(tail(&capture, OutputStream::Stdout), b"2345");
        assert_eq!(tail(&capture, OutputStream::Stderr), b"abc");
        assert_eq!(capture.accepted_bytes(), 8);
        assert_eq!(capture.retained_bytes(), 7);
    }

    #[test]
    fn should_preserve_stderr_budget_during_a_stdout_flood() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(4, 4, 64).unwrap());
        capture.append(OutputStream::Stderr, b"error").unwrap();
        capture
            .append(OutputStream::Stdout, b"01234567890123456789")
            .unwrap();
        assert_eq!(tail(&capture, OutputStream::Stderr), b"rror");
        assert_eq!(tail(&capture, OutputStream::Stdout), b"6789");
        assert_eq!(capture.retained_bytes(), 8);
    }

    #[test]
    fn should_keep_tails_in_order_across_ring_wraps() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(5, 0, 64).unwrap());
        for chunk in [b"ab".as_slice(), b"cd", b"ef", b"gh"] {
            capture.append(OutputStream::Stdout, chunk).unwrap();
        }
        assert_eq!(tail(&capture, OutputStream::Stdout), b"defgh");
        assert_eq!(
            capture.stats(OutputStream::Stdout),
            StreamCaptureStats {
                accepted_bytes: 8,
                retained_bytes: 5,
                discarded_tail_bytes: 3
            }
        );
    }

    #[test]
    fn should_accept_no_more_bytes_after_the_budget_is_exhausted() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(2, 2, 4).unwrap());
        capture.append(OutputStream::Stdout, b"abcd").unwrap();
        for _ in 0..1000 {
            assert_eq!(
                capture.append(OutputStream::Stderr, b"rejected").unwrap(),
                CapturedChunk {
                    accepted_bytes: 0,
                    rejected_bytes: 8,
                    budget_exhausted: true
                }
            );
        }
        assert_eq!(capture.accepted_bytes(), 4);
        assert_eq!(
            capture.stats(OutputStream::Stderr),
            StreamCaptureStats::default()
        );
        assert_eq!(tail(&capture, OutputStream::Stdout), b"cd");
        assert_eq!(capture.stderr.bytes.capacity(), 0);
    }

    #[test]
    fn should_preserve_binary_bytes_without_decoding_or_sanitizing_them() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(4, 0, 8).unwrap());
        capture
            .append(OutputStream::Stdout, &[0, 0xff, 0x1b, b'x'])
            .unwrap();
        assert_eq!(tail(&capture, OutputStream::Stdout), &[0, 0xff, 0x1b, b'x']);
    }

    #[test]
    fn should_allocate_no_payload_for_idle_or_count_only_streams() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(0, 0, 8).unwrap());
        capture.append(OutputStream::Stdout, b"count").unwrap();
        assert_eq!(capture.retained_bytes(), 0);
        assert_eq!(capture.stdout.bytes.capacity(), 0);
        assert_eq!(capture.stderr.bytes.capacity(), 0);
        assert_eq!(capture.stats(OutputStream::Stdout).discarded_tail_bytes, 5);
    }

    #[test]
    fn should_validate_explicit_limits_and_ceiling_overflow() {
        for (out, err, total) in [
            (1, 0, 0),
            (4, 4, 7),
            (usize::MAX, 1, 8),
            (MAX_OUTPUT_TAIL_BYTES + 1, 0, MAX_OUTPUT_CAPTURE_BYTES),
            (0, 0, MAX_OUTPUT_CAPTURE_BYTES + 1),
        ] {
            assert_eq!(
                OutputCapturePolicy::new(out, err, total),
                Err(OutputCaptureError::InvalidPolicy)
            );
        }
    }

    #[test]
    fn should_not_allocate_or_consume_budget_for_empty_chunks() {
        let mut capture = ProcessOutputCapture::new(OutputCapturePolicy::new(4, 4, 8).unwrap());
        let result = capture.append(OutputStream::Stdout, b"").unwrap();
        assert_eq!(
            result,
            CapturedChunk {
                accepted_bytes: 0,
                rejected_bytes: 0,
                budget_exhausted: false
            }
        );
        assert_eq!(capture.stdout.bytes.capacity(), 0);
        assert_eq!(capture.stderr.bytes.capacity(), 0);
        assert_eq!(capture.accepted_bytes(), 0);
    }

    #[test]
    fn should_match_reference_tails_across_interleaved_chunk_boundaries() {
        for chunk_size in 1..=17 {
            let mut capture =
                ProcessOutputCapture::new(OutputCapturePolicy::new(5, 3, 57).unwrap());
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut accepted_total = 0;
            for index in 0..20 {
                let chunk = vec![index as u8; chunk_size];
                let stream = if index % 2 == 0 {
                    OutputStream::Stdout
                } else {
                    OutputStream::Stderr
                };
                let accepted = chunk_size.min(57 - accepted_total);
                let (reference, limit) = match stream {
                    OutputStream::Stdout => (&mut stdout, 5),
                    OutputStream::Stderr => (&mut stderr, 3),
                };
                reference.extend_from_slice(&chunk[..accepted]);
                let drop_prefix = reference.len().saturating_sub(limit);
                reference.drain(..drop_prefix);
                let result = capture.append(stream, &chunk).unwrap();
                accepted_total += accepted;
                assert_eq!(result.accepted_bytes, accepted);
                assert_eq!(capture.accepted_bytes(), accepted_total as u64);
                assert_eq!(tail(&capture, OutputStream::Stdout), stdout);
                assert_eq!(tail(&capture, OutputStream::Stderr), stderr);
                assert!(capture.retained_bytes() <= 8);
            }
        }
    }
}
