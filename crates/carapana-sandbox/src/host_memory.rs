//! Read-only host and visible inherited cgroup signals, not RSS enforcement.
//! No calibrated defaults; a snapshot cannot reserve memory against unrelated applications.

use std::{fmt, fs::File, io::Read};
mod cgroup;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostMemoryError {
    InvalidPolicy,
    Unavailable,
    InvalidSample,
    InsufficientAvailable,
    PressureExceeded,
}
impl fmt::Display for HostMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPolicy => "invalid explicit host memory admission policy",
            Self::Unavailable => "host memory admission signals unavailable",
            Self::InvalidSample => "host memory admission signals invalid",
            Self::InsufficientAvailable => "host memory available is below the admission margin",
            Self::PressureExceeded => "host memory pressure exceeds the admission policy",
        })
    }
}
impl std::error::Error for HostMemoryError {}

/// Explicit operator-controlled limits. Percentages are basis points: 10000 means 100.00%.
/// This policy has no default and does not authorize execution or widen a cgroup limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMemoryAdmissionPolicy {
    min_available_after_bytes: u64,
    max_some_avg10_bps: u16,
    max_full_avg10_bps: u16,
}
impl HostMemoryAdmissionPolicy {
    pub fn new(
        min_available_after_bytes: u64,
        max_some_avg10_bps: u16,
        max_full_avg10_bps: u16,
    ) -> Result<Self, HostMemoryError> {
        if max_some_avg10_bps > 10000 || max_full_avg10_bps > 10000 {
            return Err(HostMemoryError::InvalidPolicy);
        }
        Ok(Self {
            min_available_after_bytes,
            max_some_avg10_bps,
            max_full_avg10_bps,
        })
    }
    pub fn min_available_after_bytes(&self) -> u64 {
        self.min_available_after_bytes
    }
    pub fn max_some_avg10_bps(&self) -> u16 {
        self.max_some_avg10_bps
    }
    pub fn max_full_avg10_bps(&self) -> u16 {
        self.max_full_avg10_bps
    }

    /// Re-read trusted fixed kernel paths on every call; no caller-supplied samples or paths.
    pub fn check(&self, additional_bytes: u64) -> Result<(), HostMemoryError> {
        let meminfo = read_bounded(
            File::open("/proc/meminfo").map_err(|_| HostMemoryError::Unavailable)?,
            64 * 1024,
        )?;
        let pressure = read_bounded(
            File::open("/proc/pressure/memory").map_err(|_| HostMemoryError::Unavailable)?,
            4096,
        )?;
        let mut sample = parse_sample(&meminfo, &pressure)?;
        let controller = cgroup::read()?;
        if let Some(available) = controller.available_bytes {
            sample.available_bytes = sample.available_bytes.min(available);
        }
        sample.some_avg10_bps = sample.some_avg10_bps.max(controller.some_avg10_bps);
        sample.full_avg10_bps = sample.full_avg10_bps.max(controller.full_avg10_bps);
        self.evaluate(sample, additional_bytes)
    }

    fn evaluate(
        &self,
        sample: HostMemorySample,
        additional_bytes: u64,
    ) -> Result<(), HostMemoryError> {
        let needed = self
            .min_available_after_bytes
            .checked_add(additional_bytes)
            .ok_or(HostMemoryError::InsufficientAvailable)?;
        if sample.available_bytes < needed {
            return Err(HostMemoryError::InsufficientAvailable);
        }
        if sample.some_avg10_bps > self.max_some_avg10_bps
            || sample.full_avg10_bps > self.max_full_avg10_bps
        {
            return Err(HostMemoryError::PressureExceeded);
        }
        Ok(())
    }
}

fn read_bounded(reader: impl Read, limit: u64) -> Result<String, HostMemoryError> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HostMemoryError::Unavailable)?;
    if bytes.len() as u64 > limit {
        return Err(HostMemoryError::InvalidSample);
    }
    String::from_utf8(bytes).map_err(|_| HostMemoryError::InvalidSample)
}

struct HostMemorySample {
    available_bytes: u64,
    some_avg10_bps: u16,
    full_avg10_bps: u16,
}

fn parse_sample(meminfo: &str, pressure: &str) -> Result<HostMemorySample, HostMemoryError> {
    let mut total = None;
    let mut available = None;
    for line in meminfo.lines() {
        let mut fields = line.split_ascii_whitespace();
        let slot = match fields.next() {
            Some("MemTotal:") => &mut total,
            Some("MemAvailable:") => &mut available,
            _ => continue,
        };
        let kb = unsigned(fields.next().ok_or(HostMemoryError::InvalidSample)?)?;
        if fields.next() != Some("kB") || fields.next().is_some() || slot.is_some() {
            return Err(HostMemoryError::InvalidSample);
        }
        *slot = Some(kb.checked_mul(1024).ok_or(HostMemoryError::InvalidSample)?);
    }
    let total = total.ok_or(HostMemoryError::InvalidSample)?;
    let available_bytes = available.ok_or(HostMemoryError::InvalidSample)?;
    if total == 0 || available_bytes > total {
        return Err(HostMemoryError::InvalidSample);
    }
    let (some_avg10_bps, full_avg10_bps) = parse_pressure(pressure)?;
    Ok(HostMemorySample {
        available_bytes,
        some_avg10_bps,
        full_avg10_bps,
    })
}

fn parse_pressure(pressure: &str) -> Result<(u16, u16), HostMemoryError> {
    let mut some = None;
    let mut full = None;
    for line in pressure.lines() {
        let mut fields = line.split_ascii_whitespace();
        let slot = match fields.next() {
            Some("some") => &mut some,
            Some("full") => &mut full,
            _ => return Err(HostMemoryError::InvalidSample),
        };
        if slot.is_some() {
            return Err(HostMemoryError::InvalidSample);
        }
        let mut values = [None; 4];
        for field in fields {
            let (key, value) = field
                .split_once('=')
                .ok_or(HostMemoryError::InvalidSample)?;
            let (index, value) = match key {
                "avg10" => (0, u64::from(basis_points(value)?)),
                "avg60" => (1, u64::from(basis_points(value)?)),
                "avg300" => (2, u64::from(basis_points(value)?)),
                "total" => (3, unsigned(value)?),
                _ => return Err(HostMemoryError::InvalidSample),
            };
            if values[index].replace(value).is_some() {
                return Err(HostMemoryError::InvalidSample);
            }
        }
        if values.iter().any(Option::is_none) {
            return Err(HostMemoryError::InvalidSample);
        }
        *slot = Some(
            u16::try_from(values[0].ok_or(HostMemoryError::InvalidSample)?)
                .map_err(|_| HostMemoryError::InvalidSample)?,
        );
    }
    Ok((
        some.ok_or(HostMemoryError::InvalidSample)?,
        full.ok_or(HostMemoryError::InvalidSample)?,
    ))
}

fn unsigned(value: &str) -> Result<u64, HostMemoryError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HostMemoryError::InvalidSample);
    }
    value.parse().map_err(|_| HostMemoryError::InvalidSample)
}

fn basis_points(value: &str) -> Result<u16, HostMemoryError> {
    let (whole, fraction) = value
        .split_once('.')
        .ok_or(HostMemoryError::InvalidSample)?;
    if fraction.len() != 2 {
        return Err(HostMemoryError::InvalidSample);
    }
    let bps = unsigned(whole)?
        .checked_mul(100)
        .and_then(|whole| whole.checked_add(unsigned(fraction).ok()?))
        .ok_or(HostMemoryError::InvalidSample)?;
    if bps > 10000 {
        return Err(HostMemoryError::InvalidSample);
    }
    u16::try_from(bps).map_err(|_| HostMemoryError::InvalidSample)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MEMORY: &str = "MemTotal: 4096 kB\nMemFree: 1 kB\nMemAvailable: 2048 kB\n";
    const PSI: &str = "some avg10=1.25 avg60=0.25 avg300=0.00 total=999\nfull avg10=0.50 avg60=0.00 avg300=0.00 total=22\n";

    #[test]
    fn should_use_available_not_free_or_total_and_check_exact_policy_boundaries() {
        let policy = HostMemoryAdmissionPolicy::new(1024, 125, 50).unwrap();
        assert_eq!(
            policy.evaluate(parse_sample(MEMORY, PSI).unwrap(), 2048 * 1024 - 1024),
            Ok(())
        );
        assert_eq!(
            policy.evaluate(parse_sample(MEMORY, PSI).unwrap(), 2048 * 1024 - 1023),
            Err(HostMemoryError::InsufficientAvailable)
        );
        assert_eq!(
            policy.evaluate(parse_sample(MEMORY, PSI).unwrap(), u64::MAX),
            Err(HostMemoryError::InsufficientAvailable)
        );
        for (some, full) in [(124, 50), (125, 49)] {
            assert_eq!(
                HostMemoryAdmissionPolicy::new(0, some, full)
                    .unwrap()
                    .evaluate(parse_sample(MEMORY, PSI).unwrap(), 0),
                Err(HostMemoryError::PressureExceeded)
            );
        }
        assert_eq!(
            HostMemoryAdmissionPolicy::new(0, 10001, 0),
            Err(HostMemoryError::InvalidPolicy)
        );
        assert_eq!(
            HostMemoryAdmissionPolicy::new(0, 0, 10001),
            Err(HostMemoryError::InvalidPolicy)
        );
    }

    #[test]
    fn should_reject_missing_duplicate_wrong_unit_and_overflow_memory_values() {
        for input in [
            "",
            "MemTotal: 1 kB\n",
            "MemTotal: 0 kB\nMemAvailable: 0 kB\n",
            "MemTotal: 1 kB\nMemAvailable: 2 kB\n",
            "MemTotal: 4096 bytes\nMemAvailable: 1 kB\n",
            "MemTotal: 4096 kB\nMemAvailable: 1 kB\nMemAvailable: 1 kB\n",
            "MemTotal: 18446744073709551615 kB\nMemAvailable: 1 kB\n",
            "MemTotal: +4096 kB\nMemAvailable: 1 kB\n",
        ] {
            assert!(matches!(
                parse_sample(input, PSI),
                Err(HostMemoryError::InvalidSample)
            ));
        }
    }

    #[test]
    fn should_reject_invalid_or_incomplete_pressure_instead_of_assuming_zero() {
        for value in [
            "NaN",
            "inf",
            "-0.01",
            "100.01",
            "1",
            "0.001",
            "18446744073709551615.00",
        ] {
            assert!(matches!(
                parse_sample(MEMORY, &PSI.replace("1.25", value)),
                Err(HostMemoryError::InvalidSample)
            ));
        }
        for input in [
            "".to_owned(),
            PSI.replace("full ", "some "),
            PSI.replace(" avg60=0.25", ""),
            PSI.replace("total=999", "total=999 total=999"),
            PSI.replace("total=999", "total=+999"),
            PSI.replace("some ", "other "),
        ] {
            assert!(matches!(
                parse_sample(MEMORY, &input),
                Err(HostMemoryError::InvalidSample)
            ));
        }
    }

    #[test]
    fn should_bound_input_before_parsing_and_reject_invalid_utf8() {
        assert_eq!(
            read_bounded(&b"abcd"[..], 3),
            Err(HostMemoryError::InvalidSample)
        );
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), "abc");
        assert_eq!(
            read_bounded(&b"\xff"[..], 3),
            Err(HostMemoryError::InvalidSample)
        );
    }

    #[test]
    fn should_read_real_kernel_signals_without_allocating_pressure_or_selecting_defaults() {
        // Permissive test-only limits exercise the parser, not production defaults or an OOM test.
        assert_eq!(
            HostMemoryAdmissionPolicy::new(0, 10000, 10000)
                .unwrap()
                .check(0),
            Ok(())
        );
        assert_eq!(
            HostMemoryAdmissionPolicy::new(u64::MAX, 10000, 10000)
                .unwrap()
                .check(1),
            Err(HostMemoryError::InsufficientAvailable)
        );
    }
}
