//! Scoped cgroup preparation and admission for directly-owned, pre-exec gated children.
//! No spawning or host setup. The caller must authenticate and hold the child in trusted gate code.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CgroupError {
    UnsupportedPlatform,
    InvalidPath,
    InvalidLimits,
    Unavailable,
    NotCgroupV2,
    NotDelegated,
    MissingControllers,
    InvalidDomain,
    ControlRejected,
    LimitMismatch,
    IdentityChanged,
    Populated,
    CleanupFailed,
    AlreadyAdmitted,
    ChildUnavailable,
    AdmissionFailed,
    TerminationTimedOut,
}

/// Private guardian integration: validate that its reserved stderr kill switch belongs to the
/// kernel-reported cgroup of this very process, before releasing the pre-exec admission gate.
#[cfg(target_os = "linux")]
pub fn verify_guardian_resource_membership() -> Result<(), CgroupError> {
    linux::verify_guardian_membership()
}

/// Kill only the current guardian's descriptor-bound scope, including the guardian itself.
/// This is used on controller loss as well as normal cleanup. Stderr is NOT a log stream here.
#[cfg(target_os = "linux")]
pub fn terminate_guardian_resource_scope() -> Result<(), CgroupError> {
    linux::verify_guardian_membership()?;
    let count =
        rustix::io::write(std::io::stderr(), b"1").map_err(|_| CgroupError::ControlRejected)?;
    if count != 1 {
        return Err(CgroupError::ControlRejected);
    }
    Ok(())
}

/// Finite, explicit payload limits; not a machine-wide reservation or a complete resource policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CgroupLimits {
    memory_bytes: u64,
    max_pids: u32,
}

impl CgroupLimits {
    pub fn new(memory_bytes: u64, max_pids: u32) -> Result<Self, CgroupError> {
        if memory_bytes == 0
            || memory_bytes > i64::MAX as u64
            || !memory_bytes.is_multiple_of(64 * 1024)
            || max_pids == 0
        {
            return Err(CgroupError::InvalidLimits);
        }
        Ok(Self {
            memory_bytes,
            max_pids,
        })
    }

    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
    pub fn max_pids(&self) -> u32 {
        self.max_pids
    }
}

/// An explicitly selected parent, pinned by descriptor. No automatic discovery or host reconfiguration.
pub struct DelegatedCgroup {
    #[cfg(target_os = "linux")]
    directory: std::fs::File,
}

impl DelegatedCgroup {
    /// Read-only validation. The parent must be pre-delegated and memory/pids enabled by its owner.
    pub fn open(parent: &Path) -> Result<Self, CgroupError> {
        #[cfg(target_os = "linux")]
        {
            linux::open_parent(parent)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = parent;
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Create and configure only a fresh empty child. Does not admit any process.
    pub fn prepare(&mut self, limits: CgroupLimits) -> Result<PreparedCgroup<'_>, CgroupError> {
        #[cfg(target_os = "linux")]
        {
            linux::prepare(self, limits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = limits;
            Err(CgroupError::UnsupportedPlatform)
        }
    }
}

/// A configured group with one-shot admission, not an execution permission.
/// No arbitrary PID attachment is exposed. The supervisor must establish a trusted gate before exec,
/// retain the lease while workloads run and reap its owned Child after terminating the cgroup.
///
/// ```compile_fail
/// fn log(group: &carapana_sandbox::delegated_cgroup::PreparedCgroup<'_>) {
///     let _unsafe_log = format!("{group:?}");
/// }
/// ```
pub struct PreparedCgroup<'parent> {
    #[cfg(target_os = "linux")]
    parent: &'parent DelegatedCgroup,
    #[cfg(not(target_os = "linux"))]
    _parent: std::marker::PhantomData<&'parent DelegatedCgroup>,
    #[cfg(target_os = "linux")]
    directory: std::fs::File,
    #[cfg(target_os = "linux")]
    name: String,
    limits: CgroupLimits,
    #[cfg(target_os = "linux")]
    removed: bool,
    #[cfg(target_os = "linux")]
    admitted: bool,
}

impl PreparedCgroup<'_> {
    pub fn limits(&self) -> CgroupLimits {
        self.limits
    }

    pub fn verify_ready_for_admission(&self) -> Result<(), CgroupError> {
        #[cfg(target_os = "linux")]
        {
            if self.admitted {
                return Err(CgroupError::AlreadyAdmitted);
            }
            self.verify_empty()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Dedicated guardian-only kill switch. Install as its stderr using Command's safe stdio mapping;
    /// target stderr must be replaced by a capture pipe. This is not a diagnostic/output stream.
    pub fn guardian_kill_stdio(&self) -> Result<std::process::Stdio, CgroupError> {
        #[cfg(target_os = "linux")]
        {
            self.verify_ready_for_admission()?;
            linux::guardian_kill_stdio(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Admit only a directly-owned child still blocked in trusted application code before exec/fork.
    /// The caller must establish that gate; this API does not make an already-running command safe,
    /// migrate existing descendants or retroactively account memory charged before migration.
    /// Accepting Child, not a PID, retains ownership and prevents PID reuse before a write.
    pub fn admit_blocked_child(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<(), CgroupError> {
        #[cfg(target_os = "linux")]
        {
            linux::admit(self, child)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = child;
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Terminate this lease's admitted group, including descendants that changed process groups.
    /// No kill is allowed before an admission attempt; an unexpected occupied unadmitted group fails
    /// closed. Reaping the owned Child remains the supervisor's responsibility after this method.
    pub fn terminate_and_wait(&mut self, timeout: std::time::Duration) -> Result<(), CgroupError> {
        #[cfg(target_os = "linux")]
        {
            linux::terminate(self, timeout)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = timeout;
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Revalidate the same empty cgroup and its exact configured controls.
    pub fn verify_empty(&self) -> Result<(), CgroupError> {
        #[cfg(target_os = "linux")]
        {
            linux::verify(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CgroupError::UnsupportedPlatform)
        }
    }

    /// Remove only this empty child, never recursively or by a supplied arbitrary name.
    pub fn release(self) -> Result<(), CgroupError> {
        #[cfg(target_os = "linux")]
        {
            let mut group = self;
            linux::remove(&mut group)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CgroupError::UnsupportedPlatform)
        }
    }
}

impl Drop for PreparedCgroup<'_> {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            // Only a group explicitly admitted by this lease is eligible for fail-closed termination.
            // An empty, unadmitted preparation retains its original no-kill behavior.
            if self.admitted && !self.removed {
                let _ = linux::terminate(self, std::time::Duration::from_secs(3));
            }
            let _ = linux::remove(self);
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::fs::{AtFlags, Mode, OFlags, fstatfs, mkdirat, open, openat, unlinkat};
    use std::{
        fs::File,
        io::{Read, Write},
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    // Authoritative cgroup v2 filesystem magic, documented by the kernel admin guide.
    const CGROUP2_MAGIC: u64 = 0x6367_7270;
    const CONTROL_BYTES: u64 = 4096;

    pub(super) fn guardian_kill_stdio(
        group: &PreparedCgroup<'_>,
    ) -> Result<std::process::Stdio, CgroupError> {
        verify_identity(group)?;
        let fd = openat(
            &group.directory,
            "cgroup.kill",
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CgroupError::ControlRejected)?;
        Ok(std::process::Stdio::from(File::from(fd)))
    }

    pub(super) fn verify_guardian_membership() -> Result<(), CgroupError> {
        use std::{fs, path::Component};
        let stderr = std::io::stderr();
        if fstatfs(&stderr)
            .map_err(|_| CgroupError::ControlRejected)?
            .f_type as u64
            != CGROUP2_MAGIC
        {
            return Err(CgroupError::NotCgroupV2);
        }
        let kill_path =
            fs::read_link("/proc/self/fd/2").map_err(|_| CgroupError::ControlRejected)?;
        if kill_path.file_name() != Some(std::ffi::OsStr::new("cgroup.kill")) {
            return Err(CgroupError::ControlRejected);
        }
        let kill_parent = kill_path.parent().ok_or(CgroupError::ControlRejected)?;
        let mut membership = String::new();
        File::open("/proc/self/cgroup")
            .map_err(|_| CgroupError::ControlRejected)?
            .take(CONTROL_BYTES + 1)
            .read_to_string(&mut membership)
            .map_err(|_| CgroupError::ControlRejected)?;
        if membership.len() as u64 > CONTROL_BYTES {
            return Err(CgroupError::ControlRejected);
        }
        let mut paths = membership
            .lines()
            .filter_map(|line| line.strip_prefix("0::"));
        let path = paths.next().ok_or(CgroupError::ControlRejected)?;
        if paths.next().is_some()
            || !Path::new(path).is_absolute()
            || Path::new(path)
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(CgroupError::ControlRejected);
        }
        // The trusted guardian remains outside workload namespaces. Require the standard visible
        // cgroup2 mount and fail closed on unavailable/mismatched membership rather than guessing.
        let own = fs::metadata(
            Path::new("/sys/fs/cgroup").join(
                Path::new(path)
                    .strip_prefix("/")
                    .map_err(|_| CgroupError::ControlRejected)?,
            ),
        )
        .map_err(|_| CgroupError::ControlRejected)?;
        let switch = fs::metadata(kill_parent).map_err(|_| CgroupError::ControlRejected)?;
        if !own.is_dir()
            || !switch.is_dir()
            || own.dev() != switch.dev()
            || own.ino() != switch.ino()
        {
            return Err(CgroupError::AdmissionFailed);
        }
        Ok(())
    }

    fn directory(parent: &File, name: &std::ffi::OsStr) -> Result<File, CgroupError> {
        let fd = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CgroupError::Unavailable)?;
        Ok(File::from(fd))
    }

    pub(super) fn open_parent(path: &Path) -> Result<DelegatedCgroup, CgroupError> {
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute()
            || bytes.len() > 4096
            || bytes.contains(&0)
            || !bytes.starts_with(b"/")
        {
            return Err(CgroupError::InvalidPath);
        }
        let mut depth = 0;
        for part in bytes[1..].split(|byte| *byte == b'/') {
            if part.is_empty() || part == b"." || part == b".." || part.len() > 255 {
                return Err(CgroupError::InvalidPath);
            }
            depth += 1;
            if depth > 64 {
                return Err(CgroupError::InvalidPath);
            }
        }
        let root = open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CgroupError::Unavailable)?;
        let mut current = File::from(root);
        for part in bytes[1..].split(|byte| *byte == b'/') {
            current = directory(&current, std::ffi::OsStr::from_bytes(part))?;
        }
        let info = fstatfs(&current).map_err(|_| CgroupError::Unavailable)?;
        if info.f_type as u64 != CGROUP2_MAGIC {
            return Err(CgroupError::NotCgroupV2);
        }
        validate_parent(&current)?;
        Ok(DelegatedCgroup { directory: current })
    }

    fn read_control(parent: &File, name: &str) -> Result<String, CgroupError> {
        let fd = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| CgroupError::ControlRejected)?;
        let mut value = String::new();
        File::from(fd)
            .take(CONTROL_BYTES + 1)
            .read_to_string(&mut value)
            .map_err(|_| CgroupError::ControlRejected)?;
        if value.len() as u64 > CONTROL_BYTES {
            return Err(CgroupError::ControlRejected);
        }
        Ok(value)
    }

    fn write_control(parent: &File, name: &str, value: &str) -> Result<(), CgroupError> {
        let fd = openat(
            parent,
            name,
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CgroupError::ControlRejected)?;
        // cgroup writes are commands: never retry a partial write as a second command.
        let count = File::from(fd)
            .write(value.as_bytes())
            .map_err(|_| CgroupError::ControlRejected)?;
        if count != value.len() {
            return Err(CgroupError::ControlRejected);
        }
        Ok(())
    }

    fn validate_parent(parent: &File) -> Result<(), CgroupError> {
        let metadata = parent.metadata().map_err(|_| CgroupError::Unavailable)?;
        let uid = rustix::process::geteuid();
        if uid.is_root() || metadata.uid() != uid.as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(CgroupError::NotDelegated);
        }
        if read_control(parent, "cgroup.type")?.trim() != "domain" {
            return Err(CgroupError::InvalidDomain);
        }
        for name in ["cgroup.controllers", "cgroup.subtree_control"] {
            if !controllers_ready(&read_control(parent, name)?) {
                return Err(CgroupError::MissingControllers);
            }
        }
        Ok(())
    }

    fn controllers_ready(value: &str) -> bool {
        value.split_whitespace().any(|part| part == "memory")
            && value.split_whitespace().any(|part| part == "pids")
    }

    fn is_empty(events: &str) -> Result<bool, CgroupError> {
        let mut populated = None;
        for line in events.lines() {
            let mut fields = line.split_whitespace();
            if fields.next() == Some("populated") {
                if populated.is_some() {
                    return Err(CgroupError::ControlRejected);
                }
                populated = Some(match (fields.next(), fields.next()) {
                    (Some("0"), None) => false,
                    (Some("1"), None) => true,
                    _ => return Err(CgroupError::ControlRejected),
                });
            }
        }
        populated
            .map(|value| !value)
            .ok_or(CgroupError::ControlRejected)
    }

    pub(super) fn prepare(
        parent: &mut DelegatedCgroup,
        limits: CgroupLimits,
    ) -> Result<PreparedCgroup<'_>, CgroupError> {
        validate_parent(&parent.directory)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CgroupError::Unavailable)?
            .as_nanos();
        let name = format!(
            "carapana-job-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        mkdirat(
            &parent.directory,
            name.as_str(),
            Mode::from_bits_truncate(0o700),
        )
        .map_err(|_| CgroupError::NotDelegated)?;
        let child = match directory(&parent.directory, std::ffi::OsStr::new(&name)) {
            Ok(child) => child,
            Err(error) => {
                // Only the freshly-created name; rmdir cannot remove a populated/nonempty group.
                let _ = unlinkat(&parent.directory, name.as_str(), AtFlags::REMOVEDIR);
                return Err(error);
            }
        };
        let group = PreparedCgroup {
            parent,
            directory: child,
            name,
            limits,
            removed: false,
            admitted: false,
        };
        for (control, value) in [
            ("memory.max", limits.memory_bytes.to_string()),
            ("memory.swap.max", "0".to_owned()),
            ("memory.oom.group", "1".to_owned()),
            ("pids.max", limits.max_pids.to_string()),
            ("cgroup.max.descendants", "0".to_owned()),
            ("cgroup.max.depth", "0".to_owned()),
        ] {
            write_control(&group.directory, control, &value)?;
        }
        verify(&group)?;
        Ok(group)
    }

    fn verify_identity(group: &PreparedCgroup<'_>) -> Result<(), CgroupError> {
        let current = directory(&group.parent.directory, std::ffi::OsStr::new(&group.name))?;
        let expected = group
            .directory
            .metadata()
            .map_err(|_| CgroupError::Unavailable)?;
        let current = current.metadata().map_err(|_| CgroupError::Unavailable)?;
        if current.dev() != expected.dev() || current.ino() != expected.ino() {
            return Err(CgroupError::IdentityChanged);
        }
        Ok(())
    }

    pub(super) fn verify(group: &PreparedCgroup<'_>) -> Result<(), CgroupError> {
        verify_identity(group)?;
        if !is_empty(&read_control(&group.directory, "cgroup.events")?)? {
            return Err(CgroupError::Populated);
        }
        if read_control(&group.directory, "cgroup.type")?.trim() != "domain" {
            return Err(CgroupError::InvalidDomain);
        }
        for (control, expected) in [
            ("memory.max", group.limits.memory_bytes.to_string()),
            ("memory.swap.max", "0".to_owned()),
            ("memory.oom.group", "1".to_owned()),
            ("pids.max", group.limits.max_pids.to_string()),
            ("cgroup.max.descendants", "0".to_owned()),
            ("cgroup.max.depth", "0".to_owned()),
        ] {
            if read_control(&group.directory, control)?.trim() != expected {
                return Err(CgroupError::LimitMismatch);
            }
        }
        Ok(())
    }

    pub(super) fn remove(group: &mut PreparedCgroup<'_>) -> Result<(), CgroupError> {
        if group.removed {
            return Ok(());
        }
        verify_identity(group)?;
        if !is_empty(&read_control(&group.directory, "cgroup.events")?)? {
            return Err(CgroupError::Populated);
        }
        unlinkat(
            &group.parent.directory,
            group.name.as_str(),
            AtFlags::REMOVEDIR,
        )
        .map_err(|_| CgroupError::CleanupFailed)?;
        group.removed = true;
        Ok(())
    }

    pub(super) fn admit(
        group: &mut PreparedCgroup<'_>,
        child: &mut std::process::Child,
    ) -> Result<(), CgroupError> {
        if group.admitted {
            return Err(CgroupError::AlreadyAdmitted);
        }
        verify(group)?;
        if child
            .try_wait()
            .map_err(|_| CgroupError::ChildUnavailable)?
            .is_some()
        {
            return Err(CgroupError::ChildUnavailable);
        }
        // Record before the command: on uncertain write outcome Drop must retain cleanup authority
        // for this exclusively-created cgroup. No target code may run until membership is confirmed.
        group.admitted = true;
        write_control(&group.directory, "cgroup.procs", &child.id().to_string())
            .map_err(|_| CgroupError::AdmissionFailed)?;
        let expected = child.id().to_string();
        if !read_control(&group.directory, "cgroup.procs")?
            .lines()
            .any(|line| line == expected)
        {
            return Err(CgroupError::AdmissionFailed);
        }
        Ok(())
    }

    pub(super) fn terminate(
        group: &mut PreparedCgroup<'_>,
        timeout: std::time::Duration,
    ) -> Result<(), CgroupError> {
        if timeout.is_zero() || timeout > std::time::Duration::from_secs(5) {
            return Err(CgroupError::InvalidLimits);
        }
        verify_identity(group)?;
        if !group.admitted {
            return if is_empty(&read_control(&group.directory, "cgroup.events")?)? {
                Ok(())
            } else {
                Err(CgroupError::Populated)
            };
        }
        write_control(&group.directory, "cgroup.kill", "1")?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if is_empty(&read_control(&group.directory, "cgroup.events")?)? {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(CgroupError::TerminationTimedOut);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn should_require_exact_enabled_controller_names() {
            assert!(controllers_ready("cpu memory pids io\n"));
            for invalid in ["memory", "pids", "memoryx pids", "+memory +pids", ""] {
                assert!(!controllers_ready(invalid));
            }
        }

        #[test]
        fn should_fail_closed_on_missing_duplicate_or_invalid_population_state() {
            assert_eq!(is_empty("populated 0\nfrozen 0\n"), Ok(true));
            assert_eq!(is_empty("populated 1\nfrozen 0\n"), Ok(false));
            for invalid in [
                "",
                "frozen 0\n",
                "populated 2\n",
                "populated 0\npopulated 0\n",
                "populated 0 extra\n",
            ] {
                assert_eq!(is_empty(invalid), Err(CgroupError::ControlRejected));
            }
        }

        #[test]
        fn should_reject_regular_filesystems_and_unsafe_paths_without_creating_groups() {
            assert!(matches!(
                DelegatedCgroup::open(Path::new("/var/tmp")),
                Err(CgroupError::NotCgroupV2)
            ));
            for invalid in [
                "relative",
                "/",
                "/sys//fs/cgroup",
                "/sys/../fs/cgroup",
                "/sys/fs/cgroup/",
            ] {
                assert!(matches!(
                    DelegatedCgroup::open(Path::new(invalid)),
                    Err(CgroupError::InvalidPath)
                ));
            }
        }

        #[test]
        fn should_not_treat_the_system_cgroup_root_as_a_user_delegation() {
            if Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
                assert!(matches!(
                    DelegatedCgroup::open(Path::new("/sys/fs/cgroup")),
                    Err(CgroupError::NotDelegated)
                ));
            }
        }

        #[test]
        fn should_prepare_a_real_empty_scope_only_when_a_test_parent_is_explicitly_supplied() {
            let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
                assert!(
                    std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
                    "positive cgroup gate requires an explicitly delegated CARAPANA_TEST_CGROUP_PARENT"
                );
                return;
            };
            let mut parent = DelegatedCgroup::open(Path::new(&path)).unwrap();
            let before = read_control(&parent.directory, "cgroup.subtree_control").unwrap();
            let limits = CgroupLimits::new(32 * 1024 * 1024, 8).unwrap();
            let group = parent.prepare(limits).unwrap();
            let name = group.name.clone();
            assert_eq!(group.limits(), limits);
            assert!(group.verify_empty().is_ok());
            group.release().unwrap();
            assert!(directory(&parent.directory, std::ffi::OsStr::new(&name)).is_err());
            assert_eq!(
                read_control(&parent.directory, "cgroup.subtree_control").unwrap(),
                before
            );
        }

        struct ProbeChild {
            child: std::process::Child,
            group: File,
        }

        impl ProbeChild {
            fn admit(group: &mut PreparedCgroup<'_>, script: &str) -> Self {
                use std::process::{Command, Stdio};
                let directory = group.directory.try_clone().unwrap();
                let child = Command::new("/usr/bin/python3")
                    .args(["-I", "-c", script])
                    .env_clear()
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let mut probe = Self {
                    child,
                    group: directory,
                };
                // The fixed trusted script asserts a gate byte before allocating/forking. A failed
                // admission or controller EOF never lets the synthetic workload run outside its group.
                group.admit_blocked_child(&mut probe.child).unwrap();
                probe.child.stdin.take().unwrap().write_all(b"1").unwrap();
                probe
            }

            fn wait(&mut self) -> std::process::ExitStatus {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    if let Some(status) = self.child.try_wait().unwrap() {
                        return status;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "synthetic cgroup probe exceeded deadline"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }

        impl Drop for ProbeChild {
            fn drop(&mut self) {
                // Only the freshly created test group, by pinned descriptor, not a process search.
                let _ = write_control(&self.group, "cgroup.kill", "1");
                if self.child.try_wait().ok().flatten().is_none() {
                    let _ = self.child.kill();
                }
                let _ = self.child.wait();
            }
        }

        fn event_count(value: &str, key: &str) -> u64 {
            value
                .lines()
                .find_map(|line| {
                    let (name, count) = line.split_once(' ')?;
                    if name == key {
                        count.trim().parse().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        }

        #[test]
        fn should_enforce_real_delegated_memory_and_pid_limits_when_requested() {
            let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
                assert!(
                    std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
                    "positive cgroup gate requires an explicitly delegated test parent"
                );
                return;
            };
            let mut parent = DelegatedCgroup::open(Path::new(&path)).unwrap();
            {
                let mut group = parent
                    .prepare(CgroupLimits::new(64 * 1024 * 1024, 4).unwrap())
                    .unwrap();
                let script = "import os,sys,time,errno,signal\nassert sys.stdin.buffer.read(1)==b'1'\nchildren=[]\nblocked=False\ntry:\n for _ in range(16):\n  try:\n   pid=os.fork()\n  except OSError as e:\n   assert e.errno==errno.EAGAIN\n   blocked=True\n   break\n  if pid==0:\n   time.sleep(5)\n   os._exit(0)\n  children.append(pid)\n assert blocked\nfinally:\n for pid in children:\n  os.kill(pid,signal.SIGTERM)\n for pid in children:\n  os.waitpid(pid,0)\nprint('pids-limited')";
                let mut probe = ProbeChild::admit(&mut group, script);
                assert!(probe.wait().success(), "synthetic PID-limit probe failed");
                let mut output = String::new();
                probe
                    .child
                    .stdout
                    .take()
                    .unwrap()
                    .take(128)
                    .read_to_string(&mut output)
                    .unwrap();
                assert_eq!(output.trim(), "pids-limited");
                assert!(
                    event_count(
                        &read_control(&group.directory, "pids.events").unwrap(),
                        "max"
                    ) > 0
                );
                drop(probe);
                group.verify_empty().unwrap();
                group.release().unwrap();
            }
            {
                let mut group = parent
                    .prepare(CgroupLimits::new(32 * 1024 * 1024, 4).unwrap())
                    .unwrap();
                // calloc/mmap may reserve virtual zero pages without faulting them in. Materialize
                // every page explicitly; allocation length alone is not a physical-memory workload.
                let script = "import sys\nassert sys.stdin.buffer.read(1)==b'1'\nvalue=bytearray(128*1024*1024)\nfor offset in range(0,len(value),4096): value[offset]=1\nprint(len(value))";
                let mut probe = ProbeChild::admit(&mut group, script);
                assert!(
                    !probe.wait().success(),
                    "memory probe exceeded its cgroup budget without OOM"
                );
                assert!(
                    event_count(
                        &read_control(&group.directory, "memory.events").unwrap(),
                        "oom_kill"
                    ) > 0
                );
                drop(probe);
                group.verify_empty().unwrap();
                group.release().unwrap();
            }
        }

        #[test]
        fn should_terminate_admitted_descendants_after_they_change_process_groups() {
            let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
                assert!(
                    std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
                    "positive cgroup gate requires a test parent"
                );
                return;
            };
            let mut parent = DelegatedCgroup::open(Path::new(&path)).unwrap();
            let mut group = parent
                .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
                .unwrap();
            let script = "import os,sys,time\nassert sys.stdin.buffer.read(1)==b'1'\npid=os.fork()\nif pid==0:\n os.setsid()\n assert os.getpgrp()==os.getpid()\n os.write(1,b'escaped\\n')\ntime.sleep(30)";
            let mut probe = ProbeChild::admit(&mut group, script);
            assert!(matches!(
                group.admit_blocked_child(&mut probe.child),
                Err(CgroupError::AlreadyAdmitted)
            ));
            let mut output = probe.child.stdout.take().unwrap();
            rustix::fs::fcntl_setfl(
                &output,
                rustix::fs::fcntl_getfl(&output).unwrap() | OFlags::NONBLOCK,
            )
            .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut evidence = Vec::new();
            while !evidence.windows(8).any(|part| part == b"escaped\n") {
                let mut chunk = [0_u8; 128];
                match output.read(&mut chunk) {
                    Ok(count) if count > 0 => evidence.extend_from_slice(&chunk[..count]),
                    Ok(_) => panic!("escape fixture closed before producing evidence"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("escape fixture read failed: {error}"),
                }
                assert!(evidence.len() <= 128);
                assert!(
                    std::time::Instant::now() < deadline,
                    "escape fixture did not reach setsid"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            group
                .terminate_and_wait(std::time::Duration::from_secs(3))
                .unwrap();
            assert!(!probe.wait().success());
            drop(probe);
            group.verify_empty().unwrap();
            group.release().unwrap();
        }

        #[test]
        fn should_not_kill_unadmitted_population_or_admit_an_exited_child() {
            let Some(path) = std::env::var_os("CARAPANA_TEST_CGROUP_PARENT") else {
                assert!(
                    std::env::var_os("CARAPANA_REQUIRE_CGROUP").is_none(),
                    "positive cgroup gate requires a test parent"
                );
                return;
            };
            use std::process::{Command, Stdio};
            let mut parent = DelegatedCgroup::open(Path::new(&path)).unwrap();
            let mut group = parent
                .prepare(CgroupLimits::new(64 * 1024 * 1024, 8).unwrap())
                .unwrap();
            let mut exited = Command::new("/usr/bin/python3")
                .args(["-I", "-c", "pass"])
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            assert!(exited.wait().unwrap().success());
            assert!(matches!(
                group.admit_blocked_child(&mut exited),
                Err(CgroupError::ChildUnavailable)
            ));
            let child = Command::new("/usr/bin/python3")
                .args([
                    "-I",
                    "-c",
                    "import sys; assert sys.stdin.buffer.read(1)==b'1'",
                ])
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut probe = ProbeChild {
                child,
                group: group.directory.try_clone().unwrap(),
            };
            // Inject unexpected population without granting this lease admission ownership.
            write_control(
                &group.directory,
                "cgroup.procs",
                &probe.child.id().to_string(),
            )
            .unwrap();
            assert!(matches!(
                group.terminate_and_wait(std::time::Duration::from_secs(1)),
                Err(CgroupError::Populated)
            ));
            assert!(probe.child.try_wait().unwrap().is_none());
            drop(probe); // The explicit test guard, not the unadmitted lease, owns cleanup of injection.
            group.verify_empty().unwrap();
            group.release().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_require_finite_explicit_page_aligned_limits() {
        for (memory, pids) in [(0, 1), (1, 1), (65535, 1), (65536, 0), (u64::MAX, 1)] {
            assert_eq!(
                CgroupLimits::new(memory, pids),
                Err(CgroupError::InvalidLimits)
            );
        }
        let limits = CgroupLimits::new(65536, 2).unwrap();
        assert_eq!(limits.memory_bytes(), 65536);
        assert_eq!(limits.max_pids(), 2);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn should_refuse_unsupported_platforms() {
        assert!(matches!(
            DelegatedCgroup::open(Path::new("/sys/fs/cgroup/test")),
            Err(CgroupError::UnsupportedPlatform)
        ));
    }
}
