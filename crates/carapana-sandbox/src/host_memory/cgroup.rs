//! Visible inherited cgroup v2 headroom. Fixed local paths, read-only descriptor traversal.

use super::{HostMemoryError, parse_pressure, read_bounded, unsigned};
use rustix::fs::{Mode, OFlags, fstatfs, open, openat};
use std::fs::File;

const CGROUP2_MAGIC: u64 = 0x6367_7270;

pub(super) struct ControllerMemory {
    pub(super) available_bytes: Option<u64>,
    pub(super) some_avg10_bps: u16,
    pub(super) full_avg10_bps: u16,
}

pub(super) fn read() -> Result<ControllerMemory, HostMemoryError> {
    let membership = own_membership()?;
    let parts = membership_parts(&membership)?;
    let root = open(
        "/sys/fs/cgroup",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| HostMemoryError::Unavailable)?;
    let mut current = File::from(root);
    verify_filesystem(&current)?;
    verify_global_root(&current)?;
    let mut result = ControllerMemory {
        available_bytes: None,
        some_avg10_bps: 0,
        full_avg10_bps: 0,
    };
    // The cgroup v2 root has no memory.current/max/high interface; host signals cover that level.
    for part in parts {
        current = File::from(
            openat(
                &current,
                part,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| HostMemoryError::Unavailable)?,
        );
        verify_filesystem(&current)?;
        accumulate(&mut result, read_level(&current)?);
    }
    if own_membership()? != membership {
        return Err(HostMemoryError::InvalidSample);
    }
    Ok(result)
}

fn own_membership() -> Result<String, HostMemoryError> {
    read_bounded(
        File::open("/proc/self/cgroup").map_err(|_| HostMemoryError::Unavailable)?,
        8192,
    )
}

fn membership_parts(membership: &str) -> Result<Vec<&str>, HostMemoryError> {
    let mut lines = membership.lines();
    let path = lines
        .next()
        .and_then(|line| line.strip_prefix("0::/"))
        .ok_or(HostMemoryError::InvalidSample)?;
    if lines.next().is_some() || path.len() > 4095 {
        return Err(HostMemoryError::InvalidSample);
    }
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > 255
            || part
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_control())
            || parts.len() >= 64
        {
            return Err(HostMemoryError::InvalidSample);
        }
        parts.push(part);
    }
    Ok(parts)
}

fn verify_filesystem(directory: &File) -> Result<(), HostMemoryError> {
    if fstatfs(directory)
        .map_err(|_| HostMemoryError::Unavailable)?
        .f_type as u64
        != CGROUP2_MAGIC
    {
        return Err(HostMemoryError::Unavailable);
    }
    Ok(())
}

fn verify_global_root(directory: &File) -> Result<(), HostMemoryError> {
    // The real hierarchy root has no memory.current. A namespaced/bind-mounted subtree root
    // does: its parent budgets are invisible, so never treat it as an unlimited global root.
    match openat(
        directory,
        "memory.current",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(HostMemoryError::Unavailable),
    }
}

fn control(directory: &File, name: &str) -> Result<String, HostMemoryError> {
    let fd = openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| HostMemoryError::Unavailable)?;
    read_bounded(File::from(fd), 4096)
}

fn limit(value: &str) -> Result<Option<u64>, HostMemoryError> {
    if value.trim() == "max" {
        return Ok(None);
    }
    unsigned(value.trim()).map(Some)
}

fn read_level(directory: &File) -> Result<ControllerMemory, HostMemoryError> {
    let current = unsigned(control(directory, "memory.current")?.trim())?;
    let max = limit(&control(directory, "memory.max")?)?;
    let high = limit(&control(directory, "memory.high")?)?;
    let available_bytes = match (max, high) {
        (Some(max), Some(high)) => Some(max.min(high).saturating_sub(current)),
        (Some(limit), None) | (None, Some(limit)) => Some(limit.saturating_sub(current)),
        (None, None) => None,
    };
    let (some_avg10_bps, full_avg10_bps) = parse_pressure(&control(directory, "memory.pressure")?)?;
    Ok(ControllerMemory {
        available_bytes,
        some_avg10_bps,
        full_avg10_bps,
    })
}

fn accumulate(total: &mut ControllerMemory, level: ControllerMemory) {
    total.available_bytes = match (total.available_bytes, level.available_bytes) {
        (Some(total), Some(level)) => Some(total.min(level)),
        (Some(total), None) => Some(total),
        (None, Some(level)) => Some(level),
        (None, None) => None,
    };
    total.some_avg10_bps = total.some_avg10_bps.max(level.some_avg10_bps);
    total.full_avg10_bps = total.full_avg10_bps.max(level.full_avg10_bps);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, symlink},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
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
                "carapana-memory-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            let fixture = Self(path);
            fixture.write("memory.current", "30\n");
            fixture.write("memory.max", "100\n");
            fixture.write("memory.high", "80\n");
            fixture.write("memory.pressure", "some avg10=1.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.25 avg60=0.00 avg300=0.00 total=1\n");
            fixture
        }
        fn write(&self, name: &str, value: &str) {
            fs::write(self.0.join(name), value).unwrap();
        }
        fn directory(&self) -> File {
            File::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn should_reject_non_unified_ambiguous_or_escaping_membership() {
        for input in [
            "",
            "1:memory:/unit\n",
            "0::relative\n",
            "0::/../unit\n",
            "0::/a/./b\n",
            "0::/a//b\n",
            "0::/a/\n",
            "0::/a\0b\n",
            "0::/a\n0::/b\n",
        ] {
            assert!(
                membership_parts(input).is_err(),
                "membership must be rejected"
            );
        }
        assert_eq!(membership_parts("0::/a/b\n").unwrap(), vec!["a", "b"]);
        assert!(membership_parts("0::/\n").unwrap().is_empty());
        assert!(membership_parts(&format!("0::/{}", "a/".repeat(65))).is_err());
        assert!(membership_parts(&format!("0::/{}", "a".repeat(256))).is_err());
    }

    #[test]
    fn should_read_high_and_max_without_underflow_or_treating_max_as_zero() {
        let fixture = Fixture::new();
        assert_eq!(
            read_level(&fixture.directory()).unwrap().available_bytes,
            Some(50)
        );
        fixture.write("memory.high", "max\n");
        assert_eq!(
            read_level(&fixture.directory()).unwrap().available_bytes,
            Some(70)
        );
        fixture.write("memory.max", "max\n");
        assert_eq!(
            read_level(&fixture.directory()).unwrap().available_bytes,
            None
        );
        fixture.write("memory.high", "0\n");
        assert_eq!(
            read_level(&fixture.directory()).unwrap().available_bytes,
            Some(0)
        );
        fixture.write("memory.high", "bad\n");
        assert!(matches!(
            read_level(&fixture.directory()),
            Err(HostMemoryError::InvalidSample)
        ));
        fixture.write("memory.current", "18446744073709551616\n");
        assert!(matches!(
            read_level(&fixture.directory()),
            Err(HostMemoryError::InvalidSample)
        ));
    }

    #[test]
    fn should_keep_the_tightest_ancestor_and_highest_pressure() {
        let fixture = Fixture::new();
        let mut result = ControllerMemory {
            available_bytes: None,
            some_avg10_bps: 0,
            full_avg10_bps: 0,
        };
        accumulate(&mut result, read_level(&fixture.directory()).unwrap());
        fixture.write("memory.high", "max\n");
        fixture.write("memory.max", "max\n");
        accumulate(&mut result, read_level(&fixture.directory()).unwrap());
        assert_eq!(result.available_bytes, Some(50));
        fixture.write("memory.high", "40\n");
        accumulate(&mut result, read_level(&fixture.directory()).unwrap());
        assert_eq!(result.available_bytes, Some(10));
        assert_eq!((result.some_avg10_bps, result.full_avg10_bps), (100, 25));
        assert_eq!(
            verify_filesystem(&fixture.directory()),
            Err(HostMemoryError::Unavailable)
        );
        assert_eq!(
            verify_global_root(&fixture.directory()),
            Err(HostMemoryError::Unavailable)
        );
        fs::remove_file(fixture.0.join("memory.current")).unwrap();
        assert_eq!(verify_global_root(&fixture.directory()), Ok(()));
    }

    #[test]
    fn should_reject_missing_or_symlinked_controls_without_opening_external_content() {
        let fixture = Fixture::new();
        fs::remove_file(fixture.0.join("memory.max")).unwrap();
        assert!(matches!(
            read_level(&fixture.directory()),
            Err(HostMemoryError::Unavailable)
        ));
        symlink("memory.high", fixture.0.join("memory.max")).unwrap();
        assert!(matches!(
            read_level(&fixture.directory()),
            Err(HostMemoryError::Unavailable)
        ));
    }

    #[test]
    fn should_read_the_real_visible_hierarchy_without_changing_membership_or_controls() {
        let before = own_membership().unwrap();
        let observed = read().unwrap();
        assert!(observed.some_avg10_bps <= 10000);
        assert!(observed.full_avg10_bps <= 10000);
        assert_eq!(own_membership().unwrap(), before);
    }
}
