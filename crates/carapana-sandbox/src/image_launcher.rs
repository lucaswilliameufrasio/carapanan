//! Internal Linux helper: sealed runtime and selected workspace files, no host directory mounts.
//! Not a model tool or an approval/resource-admission boundary. Run only as the dedicated CLI mode.

use crate::runtime_manifest::{
    ExpectedRuntimeFile, RuntimeFileRole, RuntimeImage, RuntimeManifestPolicy,
    receive_expected_image, receive_selected_image, validate_selected_files,
};
use crate::selected_snapshots::ReservedWorkspaceImage;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CString, OsString},
    fs::File,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, net::UnixDatagram},
    },
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

const MAX_SETUP_BYTES: usize = 128 * 1024;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_SCRATCH_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLauncherError {
    InvalidRequest,
    TransportFailed,
    TimedOut,
    RuntimeRejected,
    BackendRejected,
    ExecFailed,
}

// Private wire metadata, never image contents, source paths, controller fds or approval tokens.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    version: u8,
    files: Vec<ExpectedRuntimeFile>,
    #[serde(default)]
    workspace_files: Vec<ExpectedRuntimeFile>,
    program: PathBuf,
    arguments: Vec<OsString>,
    scratch_bytes: u64,
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

impl Setup {
    fn validate(&self) -> Result<RuntimeManifestPolicy, ImageLauncherError> {
        if self.version != 1
            || self.scratch_bytes == 0
            || self.scratch_bytes > MAX_SCRATCH_BYTES
            || self.arguments.len() > 128
            || self.files.len() > 128
            || !self.files.iter().any(|entry| {
                entry.destination == self.program && entry.role == RuntimeFileRole::Executable
            })
        {
            return Err(ImageLauncherError::InvalidRequest);
        }
        let mut bytes = 0_usize;
        for argument in &self.arguments {
            let value = argument.as_encoded_bytes();
            bytes = bytes
                .checked_add(value.len())
                .ok_or(ImageLauncherError::InvalidRequest)?;
            if value.contains(&0) || bytes > MAX_ARGUMENT_BYTES {
                return Err(ImageLauncherError::InvalidRequest);
            }
        }
        validate_selected_files(&self.workspace_files)
            .map_err(|_| ImageLauncherError::InvalidRequest)?;
        RuntimeManifestPolicy::new(self.max_files, self.max_file_bytes, self.max_total_bytes)
            .map_err(|_| ImageLauncherError::InvalidRequest)
    }
}

/// Owns immutable runtime and exact command. Not Debug/Clone/serializable and not an approval.
/// Workspace input, when present, must retain its selected snapshots' shared-quota reservation.
pub struct SandboxImageLaunch<'store> {
    image: RuntimeImage,
    setup: Setup,
    workspace: Option<ReservedWorkspaceImage<'store>>,
}

impl<'store> SandboxImageLaunch<'store> {
    pub fn new(
        image: RuntimeImage,
        program: PathBuf,
        arguments: Vec<OsString>,
        scratch_bytes: u64,
        policy: RuntimeManifestPolicy,
    ) -> Result<Self, ImageLauncherError> {
        let setup = Setup {
            version: 1,
            files: image
                .files()
                .map(|file| ExpectedRuntimeFile {
                    destination: file.destination().to_owned(),
                    role: file.role(),
                    size_bytes: file.size_bytes(),
                    sha256: *file.sha256(),
                })
                .collect(),
            workspace_files: Vec::new(),
            program,
            arguments,
            scratch_bytes,
            max_files: policy.max_files(),
            max_file_bytes: policy.max_file_bytes(),
            max_total_bytes: policy.max_total_bytes(),
        };
        setup.validate()?;
        if image.files().len() > policy.max_files()
            || image.total_bytes() > policy.max_total_bytes()
            || image
                .files()
                .any(|file| file.size_bytes() > policy.max_file_bytes())
        {
            return Err(ImageLauncherError::InvalidRequest);
        }
        let launch = Self {
            image,
            setup,
            workspace: None,
        };
        launch.encoded_setup()?;
        Ok(launch)
    }

    pub fn with_workspace(
        mut self,
        workspace: ReservedWorkspaceImage<'store>,
    ) -> Result<Self, ImageLauncherError> {
        if self.workspace.is_some() {
            return Err(ImageLauncherError::InvalidRequest);
        }
        workspace
            .verify()
            .map_err(|_| ImageLauncherError::RuntimeRejected)?;
        self.setup.workspace_files = workspace
            .image
            .files()
            .map(|file| ExpectedRuntimeFile {
                destination: file.destination().to_owned(),
                role: file.role(),
                size_bytes: file.size_bytes(),
                sha256: *file.sha256(),
            })
            .collect();
        self.setup.validate()?;
        self.encoded_setup()?;
        self.workspace = Some(workspace);
        Ok(self)
    }

    pub fn program(&self) -> &std::path::Path {
        &self.setup.program
    }
    pub fn arguments(&self) -> &[OsString] {
        &self.setup.arguments
    }
    pub fn scratch_bytes(&self) -> u64 {
        self.setup.scratch_bytes
    }
    pub fn runtime_files(
        &self,
    ) -> impl ExactSizeIterator<Item = &crate::runtime_manifest::RuntimeImageFile> {
        self.image.files()
    }
    pub fn workspace(&self) -> Option<&ReservedWorkspaceImage<'store>> {
        self.workspace.as_ref()
    }

    fn encoded_setup(&self) -> Result<Vec<u8>, ImageLauncherError> {
        let bytes =
            serde_json::to_vec(&self.setup).map_err(|_| ImageLauncherError::InvalidRequest)?;
        if bytes.len() > MAX_SETUP_BYTES {
            return Err(ImageLauncherError::InvalidRequest);
        }
        Ok(bytes)
    }

    /// Send on an already-private socketpair to the dedicated helper. Caller retains lifecycle,
    /// authentication, cgroup admission and cancellation duties; this method never spawns a child.
    pub fn send(&self, socket: &UnixDatagram, deadline: Instant) -> Result<(), ImageLauncherError> {
        if let Some(workspace) = &self.workspace {
            workspace
                .verify()
                .map_err(|_| ImageLauncherError::RuntimeRejected)?;
        }
        let bytes = self.encoded_setup()?;
        loop {
            check_deadline(deadline)?;
            match rustix::net::send(
                socket,
                &bytes,
                rustix::net::SendFlags::DONTWAIT | rustix::net::SendFlags::NOSIGNAL,
            ) {
                Ok(count) if count == bytes.len() => break,
                Ok(_) => return Err(ImageLauncherError::TransportFailed),
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => idle(deadline),
                Err(_) => return Err(ImageLauncherError::TransportFailed),
            }
        }
        self.image
            .send_descriptors(socket, deadline)
            .map_err(|_| ImageLauncherError::TransportFailed)?;
        if let Some(workspace) = &self.workspace {
            workspace
                .image
                .send_selected_descriptors(socket, deadline)
                .map_err(|_| ImageLauncherError::TransportFailed)?;
        }
        Ok(())
    }
}

fn check_deadline(deadline: Instant) -> Result<(), ImageLauncherError> {
    if Instant::now() >= deadline {
        return Err(ImageLauncherError::TimedOut);
    }
    Ok(())
}
fn idle(deadline: Instant) {
    thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())));
}

/// Dedicated, single-threaded CLI mode only. On success this process becomes Bubblewrap.
/// Never call from the daemon or a shared application process: it remaps its own stdin and explicitly
/// clears CLOEXEC only on owned, validated runtime fds immediately before exec. It never maps stderr,
/// cgroup controls, approval handles, host directories or arbitrary environment into the target.
pub fn run_image_helper() -> Result<(), ImageLauncherError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let socket = UnixDatagram::from(
        rustix::io::fcntl_dupfd_cloexec(std::io::stdin(), 3)
            .map_err(|_| ImageLauncherError::TransportFailed)?,
    );
    let mut bytes = vec![0_u8; MAX_SETUP_BYTES + 1];
    let count = loop {
        check_deadline(deadline)?;
        match rustix::net::recv(
            &socket,
            &mut bytes,
            rustix::net::RecvFlags::DONTWAIT | rustix::net::RecvFlags::TRUNC,
        ) {
            Ok((count, _)) => break count,
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => idle(deadline),
            Err(_) => return Err(ImageLauncherError::TransportFailed),
        }
    };
    if count > MAX_SETUP_BYTES {
        return Err(ImageLauncherError::InvalidRequest);
    }
    let setup: Setup =
        serde_json::from_slice(&bytes[..count]).map_err(|_| ImageLauncherError::InvalidRequest)?;
    let policy = setup.validate()?;
    let image = receive_expected_image(setup.files, &socket, policy, deadline)
        .map_err(|_| ImageLauncherError::RuntimeRejected)?;
    let workspace = receive_selected_image(setup.workspace_files, &socket, deadline)
        .map_err(|_| ImageLauncherError::RuntimeRejected)?;
    // Root-owned, non-writable ancestry, NOFOLLOW and no setuid/setgid. Execute the pinned backend,
    // not a later reopening of its path. No download, PATH lookup or weaker backend fallback.
    let backend = crate::runtime_manifest::open_bubblewrap_backend()
        .map_err(|_| ImageLauncherError::BackendRejected)?;
    let mut argv = vec![CString::new("bwrap").unwrap()];
    for value in [
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--setenv",
        "LC_ALL",
        "C",
        "--setenv",
        "PATH",
        "/runtime/bin:/usr/bin:/bin",
    ] {
        argv.push(CString::new(value).unwrap());
    }
    argv.push(CString::new("--dir").unwrap());
    argv.push(CString::new("/workspace").unwrap());
    for file in image.files().chain(workspace.files()) {
        rustix::fs::seek(file.descriptor(), rustix::fs::SeekFrom::Start(0))
            .map_err(|_| ImageLauncherError::RuntimeRejected)?;
        for value in [
            OsString::from("--perms"),
            OsString::from(if file.role() == RuntimeFileRole::Executable {
                "0555"
            } else {
                "0444"
            }),
            OsString::from("--ro-bind-data"),
            OsString::from(file.descriptor().as_raw_fd().to_string()),
            file.destination().as_os_str().to_owned(),
        ] {
            argv.push(
                CString::new(value.as_bytes()).map_err(|_| ImageLauncherError::InvalidRequest)?,
            );
        }
    }
    for value in [
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--size",
        &setup.scratch_bytes.to_string(),
        "--tmpfs",
        "/tmp",
        "--chdir",
        "/workspace",
        "--remount-ro",
        "/",
        "--",
    ] {
        argv.push(CString::new(value).unwrap());
    }
    argv.push(
        CString::new(setup.program.as_os_str().as_bytes())
            .map_err(|_| ImageLauncherError::InvalidRequest)?,
    );
    for argument in setup.arguments {
        argv.push(
            CString::new(argument.as_bytes()).map_err(|_| ImageLauncherError::InvalidRequest)?,
        );
    }
    // Close the transport before exec and replace stdin; no descriptor to the private channel reaches
    // the target. Runtime descriptors alone are whitelisted, local to this helper's descriptor table.
    drop(socket);
    let null = File::open("/dev/null").map_err(|_| ImageLauncherError::ExecFailed)?;
    rustix::stdio::dup2_stdin(&null).map_err(|_| ImageLauncherError::ExecFailed)?;
    for file in image.files().chain(workspace.files()) {
        rustix::io::fcntl_setfd(file.descriptor(), rustix::io::FdFlags::empty())
            .map_err(|_| ImageLauncherError::ExecFailed)?;
    }
    match nix::unistd::fexecve(backend.as_raw_fd(), &argv, &[] as &[CString]) {
        Ok(never) => match never {},
        Err(_) => Err(ImageLauncherError::ExecFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            version: 1,
            workspace_files: Vec::new(),
            files: vec![ExpectedRuntimeFile {
                destination: PathBuf::from("/runtime/program"),
                role: RuntimeFileRole::Executable,
                size_bytes: 1,
                sha256: [0; 32],
            }],
            program: PathBuf::from("/runtime/program"),
            arguments: Vec::new(),
            scratch_bytes: 1024,
            max_files: 1,
            max_file_bytes: 1024,
            max_total_bytes: 1024,
        }
    }

    #[test]
    fn should_reject_free_form_programs_scratch_and_argument_overflow_before_exec() {
        for case in 0..7 {
            let mut setup = setup();
            match case {
                0 => setup.program = PathBuf::from("/bin/undeclared"),
                1 => setup.scratch_bytes = 0,
                2 => setup.scratch_bytes = MAX_SCRATCH_BYTES + 1,
                3 => setup.arguments = vec![OsString::from("x"); 129],
                4 => setup.arguments = vec![OsString::from("x".repeat(MAX_ARGUMENT_BYTES + 1))],
                5 => setup.arguments = vec![OsString::from("bad\0argument")],
                _ => setup.version = 2,
            }
            assert!(matches!(
                setup.validate(),
                Err(ImageLauncherError::InvalidRequest)
            ));
        }
    }
}
