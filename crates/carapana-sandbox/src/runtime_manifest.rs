//! Explicit Linux runtime file verification, without discovery, execution or mounts.
//! Manifest provenance/approval is a separate gate: a matching hash is not secret classification.

#[cfg(target_os = "linux")]
use std::fs::File;
use std::path::{Path, PathBuf};

mod image;
#[cfg(target_os = "linux")]
pub(crate) use image::{ExpectedRuntimeFile, receive_expected_image};
pub use image::{RuntimeImage, RuntimeImageFile};

#[cfg(target_os = "linux")]
pub(crate) fn open_bubblewrap_backend() -> Result<File, RuntimeManifestError> {
    linux::open_source(Path::new("/usr/bin/bwrap"), RuntimeFileRole::Executable)
}

const MAX_FILES: usize = 128;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RuntimeFileRole {
    Executable,
    Support,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeManifestError {
    UnsupportedPlatform,
    InvalidPolicy,
    InvalidPath,
    ConflictingTargets,
    MissingExecutable,
    TooManyFiles,
    ByteBudgetExceeded,
    SourceUnavailable,
    UnsafeSource,
    HashMismatch,
    SourceChanged,
    ImageUnavailable,
    ImageSealRejected,
    ImageTransportUnavailable,
    ImageTransportTimedOut,
    ImageTransportProtocol,
}

/// Explicit ceilings; no product defaults or automatic budget expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeManifestPolicy {
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

impl RuntimeManifestPolicy {
    pub fn new(
        max_files: usize,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RuntimeManifestError> {
        if max_files == 0
            || max_files > MAX_FILES
            || max_file_bytes == 0
            || max_file_bytes > max_total_bytes
            || max_total_bytes > MAX_BYTES
        {
            return Err(RuntimeManifestError::InvalidPolicy);
        }
        Ok(Self {
            max_files,
            max_file_bytes,
            max_total_bytes,
        })
    }

    pub fn max_files(&self) -> usize {
        self.max_files
    }

    pub fn max_file_bytes(&self) -> u64 {
        self.max_file_bytes
    }

    pub fn max_total_bytes(&self) -> u64 {
        self.max_total_bytes
    }
}

/// Untrusted input until verified AND approved by the caller's independent policy.
/// Intentionally not Debug/Clone/serializable: paths may disclose private host layout.
pub struct RuntimeFileSpec {
    source: PathBuf,
    destination: PathBuf,
    role: RuntimeFileRole,
    expected_sha256: [u8; 32],
}

impl RuntimeFileSpec {
    pub fn new(
        source: PathBuf,
        destination: PathBuf,
        role: RuntimeFileRole,
        expected_sha256: [u8; 32],
    ) -> Result<Self, RuntimeManifestError> {
        validate_path(&source)?;
        validate_destination(&destination)?;
        Ok(Self {
            source,
            destination,
            role,
            expected_sha256,
        })
    }

    /// Untrusted input path for explicit review, not an authorized file or mount source.
    pub fn source(&self) -> &Path {
        &self.source
    }
}

/// Read-only descriptor pinned to a verified object, not an execution capability or immutable copy.
pub struct VerifiedRuntimeFile {
    spec: RuntimeFileSpec,
    #[cfg(target_os = "linux")]
    file: File,
    #[cfg(target_os = "linux")]
    signature: linux::Signature,
    size_bytes: u64,
}

impl VerifiedRuntimeFile {
    /// Exact sandbox target; escape before displaying it.
    pub fn destination(&self) -> &Path {
        &self.spec.destination
    }
    pub fn role(&self) -> RuntimeFileRole {
        self.spec.role
    }
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    pub fn expected_sha256(&self) -> &[u8; 32] {
        &self.spec.expected_sha256
    }

    /// For a future descriptor-based mount boundary, never reopen the source by path for mounting.
    /// Revalidation and approval remain required. The descriptor does not lock out in-place writes.
    #[cfg(target_os = "linux")]
    pub fn descriptor(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.file.as_fd()
    }
}

/// Observational evidence only. No Debug/Clone/serialization or automatic dependency expansion.
///
/// ```compile_fail
/// fn log(manifest: &carapana_sandbox::runtime_manifest::VerifiedRuntimeManifest) {
///     let _unsafe_log = format!("{manifest:?}");
/// }
/// ```
pub struct VerifiedRuntimeManifest {
    files: Vec<VerifiedRuntimeFile>,
    total_bytes: u64,
    #[cfg(target_os = "linux")]
    policy: RuntimeManifestPolicy,
}

impl VerifiedRuntimeManifest {
    /// Consume mutable-source evidence into private kernel-sealed copies, within explicit ceilings.
    /// This neither approves execution nor installs mounts or transports descriptors to a child.
    pub fn freeze(
        self,
        policy: RuntimeManifestPolicy,
    ) -> Result<RuntimeImage, RuntimeManifestError> {
        image::freeze(self, policy)
    }

    /// Receive sealed descriptors matching this exact manifest over a caller-owned private socketpair.
    /// Neither the peer nor the manifest is an authenticated operator approval.
    #[cfg(target_os = "linux")]
    pub fn receive_image(
        self,
        socket: &std::os::unix::net::UnixDatagram,
        policy: RuntimeManifestPolicy,
        deadline: std::time::Instant,
    ) -> Result<RuntimeImage, RuntimeManifestError> {
        image::transport::receive(self, socket, policy, deadline)
    }

    pub fn verify(
        specs: Vec<RuntimeFileSpec>,
        policy: RuntimeManifestPolicy,
    ) -> Result<Self, RuntimeManifestError> {
        validate_manifest(&specs, policy)?;
        #[cfg(target_os = "linux")]
        {
            linux::verify(specs, policy)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(RuntimeManifestError::UnsupportedPlatform)
        }
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = &VerifiedRuntimeFile> {
        self.files.iter()
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Reopen via the same confined traversal and compare object identity, metadata and bytes.
    /// This cannot stop root modifying the object immediately after the check.
    pub fn revalidate(&self) -> Result<(), RuntimeManifestError> {
        #[cfg(target_os = "linux")]
        {
            for entry in &self.files {
                let current = linux::open_source(&entry.spec.source, entry.spec.role)?;
                if linux::Signature::capture(&current)? != entry.signature {
                    return Err(RuntimeManifestError::SourceChanged);
                }
                linux::verify_bytes(
                    &entry.file,
                    &entry.signature,
                    &entry.spec.expected_sha256,
                    self.policy.max_file_bytes,
                )?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(RuntimeManifestError::UnsupportedPlatform)
        }
    }
}

fn validate_path(path: &Path) -> Result<(), RuntimeManifestError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if !path.is_absolute() || bytes.len() > 4096 || bytes.contains(&0) || !bytes.starts_with(b"/") {
        return Err(RuntimeManifestError::InvalidPath);
    }
    let mut depth = 0;
    for component in bytes[1..].split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." || component.len() > 255
        {
            return Err(RuntimeManifestError::InvalidPath);
        }
        depth += 1;
        if depth > 32 {
            return Err(RuntimeManifestError::InvalidPath);
        }
    }
    Ok(())
}

pub(crate) fn validate_destination(path: &Path) -> Result<(), RuntimeManifestError> {
    validate_path(path)?;
    if !["/runtime", "/usr", "/bin", "/lib", "/lib64"]
        .iter()
        .any(|root| path.starts_with(root) && path != Path::new(root))
    {
        return Err(RuntimeManifestError::InvalidPath);
    }
    Ok(())
}

fn validate_manifest(
    specs: &[RuntimeFileSpec],
    policy: RuntimeManifestPolicy,
) -> Result<(), RuntimeManifestError> {
    if specs.len() > policy.max_files {
        return Err(RuntimeManifestError::TooManyFiles);
    }
    if !specs
        .iter()
        .any(|spec| spec.role == RuntimeFileRole::Executable)
    {
        return Err(RuntimeManifestError::MissingExecutable);
    }
    for (index, spec) in specs.iter().enumerate() {
        for previous in &specs[..index] {
            if spec.destination.starts_with(&previous.destination)
                || previous.destination.starts_with(&spec.destination)
            {
                return Err(RuntimeManifestError::ConflictingTargets);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::fs::{Mode, OFlags, open, openat};
    use sha2::{Digest, Sha256};
    use std::{
        ffi::OsStr,
        fs::Metadata,
        os::unix::{
            ffi::OsStrExt,
            fs::{FileExt, MetadataExt},
        },
    };

    #[derive(PartialEq, Eq)]
    pub(super) struct Signature {
        device: u64,
        inode: u64,
        size: u64,
        mode: u32,
        mtime: (i64, i64),
        ctime: (i64, i64),
    }

    impl Signature {
        pub(super) fn capture(file: &File) -> Result<Self, RuntimeManifestError> {
            let metadata = file
                .metadata()
                .map_err(|_| RuntimeManifestError::SourceUnavailable)?;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                size: metadata.len(),
                mode: metadata.mode(),
                mtime: (metadata.mtime(), metadata.mtime_nsec()),
                ctime: (metadata.ctime(), metadata.ctime_nsec()),
            })
        }
    }

    fn trusted(metadata: &Metadata, directory: bool) -> bool {
        metadata.uid() == 0
            && metadata.mode() & 0o6022 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
    }

    pub(super) fn open_source(
        path: &Path,
        role: RuntimeFileRole,
    ) -> Result<File, RuntimeManifestError> {
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let root = open("/", flags | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| RuntimeManifestError::SourceUnavailable)?;
        let mut parent = File::from(root);
        if !trusted(
            &parent
                .metadata()
                .map_err(|_| RuntimeManifestError::SourceUnavailable)?,
            true,
        ) {
            return Err(RuntimeManifestError::UnsafeSource);
        }
        let mut components = path.as_os_str().as_bytes()[1..]
            .split(|byte| *byte == b'/')
            .peekable();
        while let Some(component) = components.next() {
            let directory = components.peek().is_some();
            let entry_flags = if directory {
                flags | OFlags::DIRECTORY
            } else {
                flags
            };
            let fd = openat(
                &parent,
                OsStr::from_bytes(component),
                entry_flags,
                Mode::empty(),
            )
            .map_err(|_| RuntimeManifestError::SourceUnavailable)?;
            let file = File::from(fd);
            let metadata = file
                .metadata()
                .map_err(|_| RuntimeManifestError::SourceUnavailable)?;
            if !trusted(&metadata, directory)
                || (!directory
                    && role == RuntimeFileRole::Executable
                    && metadata.mode() & 0o111 == 0)
            {
                return Err(RuntimeManifestError::UnsafeSource);
            }
            parent = file;
        }
        Ok(parent)
    }

    pub(super) fn verify_bytes(
        file: &File,
        signature: &Signature,
        expected: &[u8; 32],
        limit: u64,
    ) -> Result<(), RuntimeManifestError> {
        if Signature::capture(file)? != *signature {
            return Err(RuntimeManifestError::SourceChanged);
        }
        if signature.size > limit {
            return Err(RuntimeManifestError::ByteBudgetExceeded);
        }
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut offset = 0_u64;
        loop {
            // Read one sentinel byte beyond the observed length, never an unbounded read_to_end.
            let requested = buffer.len().min((signature.size - offset + 1) as usize);
            let count = match file.read_at(&mut buffer[..requested], offset) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(RuntimeManifestError::SourceUnavailable),
            };
            if count == 0 {
                break;
            }
            offset += count as u64;
            if offset > signature.size {
                return Err(RuntimeManifestError::SourceChanged);
            }
            digest.update(&buffer[..count]);
        }
        if offset != signature.size || Signature::capture(file)? != *signature {
            return Err(RuntimeManifestError::SourceChanged);
        }
        let actual: [u8; 32] = digest.finalize().into();
        if actual != *expected {
            return Err(RuntimeManifestError::HashMismatch);
        }
        Ok(())
    }

    pub(super) fn verify(
        specs: Vec<RuntimeFileSpec>,
        policy: RuntimeManifestPolicy,
    ) -> Result<VerifiedRuntimeManifest, RuntimeManifestError> {
        let mut files = Vec::new();
        let mut total_bytes = 0_u64;
        for spec in specs {
            let file = open_source(&spec.source, spec.role)?;
            let signature = Signature::capture(&file)?;
            let remaining = policy.max_total_bytes - total_bytes;
            verify_bytes(
                &file,
                &signature,
                &spec.expected_sha256,
                policy.max_file_bytes.min(remaining),
            )?;
            total_bytes += signature.size;
            files.push(VerifiedRuntimeFile {
                spec,
                file,
                size_bytes: signature.size,
                signature,
            });
        }
        Ok(VerifiedRuntimeManifest {
            files,
            total_bytes,
            policy,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs,
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink},
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
                let root = std::env::temp_dir().join(format!(
                    "carapana-runtime-{}-{nonce}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(root.join("file"))
                    .unwrap();
                file.write_all(b"synthetic runtime").unwrap();
                Self(root)
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        // Only test fixtures derive a hash from current bytes. Production callers must supply an
        // independently reviewed digest; this helper never runs a host program or discovers deps.
        fn system_spec() -> RuntimeFileSpec {
            let path = fs::canonicalize("/usr/bin/true").unwrap();
            let bytes = fs::read(&path).unwrap();
            assert!(bytes.len() < 1024 * 1024);
            RuntimeFileSpec::new(
                path,
                PathBuf::from("/runtime/bin/true"),
                RuntimeFileRole::Executable,
                Sha256::digest(&bytes).into(),
            )
            .unwrap()
        }

        #[test]
        fn should_pin_only_the_explicit_system_file_and_revalidate_it_without_execution() {
            let spec = system_spec();
            let policy = RuntimeManifestPolicy::new(2, 1024 * 1024, 1024 * 1024).unwrap();
            let manifest = VerifiedRuntimeManifest::verify(vec![spec], policy).unwrap();
            assert_eq!(manifest.files().len(), 1);
            let file = manifest.files().next().unwrap();
            assert_eq!(file.destination(), Path::new("/runtime/bin/true"));
            assert_eq!(file.role(), RuntimeFileRole::Executable);
            assert_eq!(file.size_bytes(), manifest.total_bytes());
            use std::os::fd::AsRawFd;
            assert!(file.descriptor().as_raw_fd() >= 0);
            assert!(manifest.revalidate().is_ok());
        }

        #[test]
        fn should_freeze_only_verified_files_with_their_exact_metadata_and_bytes() {
            let spec = system_spec();
            let expected = fs::read(&spec.source).unwrap();
            let digest = spec.expected_sha256;
            let policy = RuntimeManifestPolicy::new(2, 1024 * 1024, 1024 * 1024).unwrap();
            let image = VerifiedRuntimeManifest::verify(vec![spec], policy)
                .unwrap()
                .freeze(policy)
                .unwrap();
            assert_eq!(image.files().len(), 1);
            assert_eq!(image.total_bytes(), expected.len() as u64);
            let entry = image.files().next().unwrap();
            assert_eq!(entry.destination(), Path::new("/runtime/bin/true"));
            assert_eq!(entry.role(), RuntimeFileRole::Executable);
            assert_eq!(entry.size_bytes(), expected.len() as u64);
            assert_eq!(entry.sha256(), &digest);
            let mut bytes = vec![0; expected.len()];
            rustix::io::pread(entry.descriptor(), &mut bytes, 0).unwrap();
            assert_eq!(bytes, expected);
            assert!(rustix::io::pwrite(entry.descriptor(), b"x", 0).is_err());
        }

        #[test]
        fn should_reject_a_freeze_budget_smaller_than_the_complete_verified_manifest() {
            let policy = RuntimeManifestPolicy::new(2, 1024 * 1024, 2 * 1024 * 1024).unwrap();
            let first = system_spec();
            let mut second = system_spec();
            second.destination = PathBuf::from("/runtime/second");
            let manifest = VerifiedRuntimeManifest::verify(vec![first, second], policy).unwrap();
            assert!(matches!(
                manifest.freeze(RuntimeManifestPolicy::new(1, 1024 * 1024, 1024 * 1024).unwrap()),
                Err(RuntimeManifestError::TooManyFiles)
            ));
            let manifest = VerifiedRuntimeManifest::verify(vec![system_spec()], policy).unwrap();
            assert!(matches!(
                manifest.freeze(RuntimeManifestPolicy::new(1, 1, 1).unwrap()),
                Err(RuntimeManifestError::ByteBudgetExceeded)
            ));
        }

        #[test]
        fn should_reject_unapproved_hash_and_file_or_shared_byte_limit() {
            let mut spec = system_spec();
            spec.expected_sha256 = [0; 32];
            assert!(matches!(
                VerifiedRuntimeManifest::verify(
                    vec![spec],
                    RuntimeManifestPolicy::new(1, 1024 * 1024, 1024 * 1024).unwrap()
                ),
                Err(RuntimeManifestError::HashMismatch)
            ));
            assert!(matches!(
                VerifiedRuntimeManifest::verify(
                    vec![system_spec()],
                    RuntimeManifestPolicy::new(1, 1, 1).unwrap()
                ),
                Err(RuntimeManifestError::ByteBudgetExceeded)
            ));
            let first = system_spec();
            let mut second = system_spec();
            second.destination = PathBuf::from("/runtime/bin/second");
            let size = fs::metadata(&first.source).unwrap().len();
            assert!(matches!(
                VerifiedRuntimeManifest::verify(
                    vec![first, second],
                    RuntimeManifestPolicy::new(2, size, size).unwrap()
                ),
                Err(RuntimeManifestError::ByteBudgetExceeded)
            ));
        }

        #[test]
        fn should_reject_writable_ancestry_symlinks_and_special_files_without_blocking() {
            let fixture = Fixture::new();
            symlink("file", fixture.0.join("linked")).unwrap();
            for path in [
                fixture.0.join("file"),
                fixture.0.join("linked"),
                PathBuf::from("/dev/null"),
            ] {
                assert!(open_source(&path, RuntimeFileRole::Support).is_err());
            }
            // /proc/self includes a symlink even though it is not a user-writable directory.
            assert!(open_source(Path::new("/proc/self/exe"), RuntimeFileRole::Executable).is_err());
            // Test a symlink below a trusted directory as well, rather than only fixtures whose
            // writable /tmp ancestor is already rejected. Do not resolve it in production.
            let alias = Path::new("/bin/true");
            if fs::symlink_metadata("/bin").unwrap().is_symlink() {
                assert!(open_source(alias, RuntimeFileRole::Executable).is_err());
            }
        }

        #[test]
        fn should_reject_permission_changes_and_metadata_content_mutations_in_pinned_files() {
            let fixture = Fixture::new();
            let path = fixture.0.join("file");
            let file = File::open(&path).unwrap();
            let signature = Signature::capture(&file).unwrap();
            let expected: [u8; 32] = Sha256::digest(b"synthetic runtime").into();
            assert!(verify_bytes(&file, &signature, &expected, 1024).is_ok());
            fs::write(&path, b"different runtime").unwrap();
            assert_eq!(
                verify_bytes(&file, &signature, &expected, 1024),
                Err(RuntimeManifestError::SourceChanged)
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
            assert!(!trusted(&file.metadata().unwrap(), false));
            assert_eq!(
                verify_bytes(&file, &signature, &expected, 1024),
                Err(RuntimeManifestError::SourceChanged)
            );
        }

        #[test]
        fn should_detect_growth_short_reads_and_not_use_a_shared_descriptor_offset() {
            let fixture = Fixture::new();
            let path = fixture.0.join("file");
            let file = File::open(&path).unwrap();
            let mut signature = Signature::capture(&file).unwrap();
            let expected: [u8; 32] = Sha256::digest(b"synthetic runtime").into();
            assert!(verify_bytes(&file, &signature, &expected, 1024).is_ok());
            assert!(verify_bytes(&file, &signature, &expected, 1024).is_ok());
            signature.size = 1;
            assert_eq!(
                verify_bytes(&file, &signature, &expected, 1024),
                Err(RuntimeManifestError::SourceChanged)
            );
            let signature = Signature::capture(&file).unwrap();
            fs::write(path, b"").unwrap();
            assert_eq!(
                verify_bytes(&file, &signature, &expected, 1024),
                Err(RuntimeManifestError::SourceChanged)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(target: &str, role: RuntimeFileRole) -> RuntimeFileSpec {
        RuntimeFileSpec::new(
            PathBuf::from("/usr/bin/example"),
            PathBuf::from(target),
            role,
            [0; 32],
        )
        .unwrap()
    }

    #[test]
    fn should_reject_noncanonical_paths_and_reserved_targets() {
        for invalid in [
            "relative",
            "/",
            "/usr/../bin/tool",
            "/usr/./bin/tool",
            "/usr//bin/tool",
            "/usr/bin/",
            "/usr/bin/x\0y",
        ] {
            assert_eq!(
                validate_path(Path::new(invalid)),
                Err(RuntimeManifestError::InvalidPath)
            );
        }
        for target in [
            "/usr",
            "/proc/runtime",
            "/workspace/tool",
            "/tmp/tool",
            "/usr-other/tool",
            "/home/tool",
        ] {
            assert!(matches!(
                RuntimeFileSpec::new(
                    PathBuf::from("/usr/bin/example"),
                    PathBuf::from(target),
                    RuntimeFileRole::Executable,
                    [0; 32]
                ),
                Err(RuntimeManifestError::InvalidPath)
            ));
        }
    }

    #[test]
    fn should_reject_duplicates_prefix_conflicts_and_manifests_without_executables() {
        let policy = RuntimeManifestPolicy::new(3, 1024, 2048).unwrap();
        for targets in [
            ["/runtime/a", "/runtime/a"],
            ["/runtime/a", "/runtime/a/b"],
            ["/runtime/a/b", "/runtime/a"],
        ] {
            let specs = vec![
                spec(targets[0], RuntimeFileRole::Executable),
                spec(targets[1], RuntimeFileRole::Support),
            ];
            assert_eq!(
                validate_manifest(&specs, policy),
                Err(RuntimeManifestError::ConflictingTargets)
            );
        }
        assert_eq!(
            validate_manifest(&[], policy),
            Err(RuntimeManifestError::MissingExecutable)
        );
        assert_eq!(
            validate_manifest(&[spec("/runtime/a", RuntimeFileRole::Support)], policy),
            Err(RuntimeManifestError::MissingExecutable)
        );
    }

    #[test]
    fn should_require_explicit_bounded_policies() {
        for (files, per_file, total) in [
            (0, 1, 1),
            (MAX_FILES + 1, 1, 1),
            (1, 0, 1),
            (1, 2, 1),
            (1, 1, MAX_BYTES + 1),
        ] {
            assert_eq!(
                RuntimeManifestPolicy::new(files, per_file, total),
                Err(RuntimeManifestError::InvalidPolicy)
            );
        }
        let policy = RuntimeManifestPolicy::new(1, 1, 1).unwrap();
        assert_eq!(
            validate_manifest(
                &[
                    spec("/runtime/a", RuntimeFileRole::Executable),
                    spec("/runtime/b", RuntimeFileRole::Support)
                ],
                policy
            ),
            Err(RuntimeManifestError::TooManyFiles)
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn should_fail_closed_on_unsupported_platforms() {
        assert!(matches!(
            VerifiedRuntimeManifest::verify(
                vec![spec("/runtime/a", RuntimeFileRole::Executable)],
                RuntimeManifestPolicy::new(1, 1, 1).unwrap()
            ),
            Err(RuntimeManifestError::UnsupportedPlatform)
        ));
    }
}
