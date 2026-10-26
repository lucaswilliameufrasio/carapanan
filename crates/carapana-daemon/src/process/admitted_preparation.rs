//! Shared nominal admission before payload creation, owned through exact approval and cleanup.
//! No model/UI authority, machine RSS guarantee, fairness or orphan-capacity reclamation.

use super::execution_approval::ReviewedSandboxSubject;
use super::*;
use carapana_core::{
    command_approval::{
        ApprovalSubject, ApprovedProposal, CommandApprovalError, CommandProposal, ProposalApproval,
    },
    trust::{TaskId, WorkspaceId},
};
use carapana_sandbox::{
    delegated_cgroup::PreparedCgroup,
    host_memory::{HostMemoryAdmissionPolicy, HostMemoryError},
    image_launcher::{MAX_SCRATCH_BYTES, MAX_SETUP_BYTES, SandboxImageLaunch},
    runtime_manifest::{
        RuntimeFileSpec, RuntimeManifestError, RuntimeManifestPolicy, VerifiedRuntimeManifest,
    },
    selected_snapshots::{
        ReservedSelectedSnapshotBundle, SelectedSnapshotPolicy, SnapshotPreparationError,
    },
};
use carapana_storage::{
    ArtifactStore, MAX_WORKSPACE_FILE_READ_BYTES, PreparationAdmissionError,
    PreparationAdmissionStore, PreparationLease, WorkspaceMetadata,
};
use std::cell::Cell;

/// Untrusted preparation intent, not approved execution. Metadata ingress must also be bounded.
/// No Debug/Clone/serialization: paths/arguments may contain private operator data.
pub struct SandboxPreparationSpec<'input> {
    pub command: &'input CommandProposal,
    pub runtime_files: Vec<RuntimeFileSpec>,
    pub runtime_policy: RuntimeManifestPolicy,
    pub selected_paths: &'input [PathBuf],
    pub selected_policy: SelectedSnapshotPolicy,
    pub scratch_bytes: u64,
    pub limits: SandboxExecutionLimits,
    pub host_memory_policy: HostMemoryAdmissionPolicy,
}
impl SandboxPreparationSpec<'_> {
    /// Conservative nominal payload reservation, not a benchmark-derived RSS claim.
    pub fn required_bytes(&self) -> Result<u64, AdmittedSandboxError> {
        if !self
            .command
            .relative_working_directory()
            .as_os_str()
            .is_empty()
            || self.scratch_bytes == 0
            || self.scratch_bytes > MAX_SCRATCH_BYTES
            || self.runtime_files.len() > self.runtime_policy.max_files()
            || self.selected_paths.len() > self.selected_policy.max_files()
        {
            return Err(AdmittedSandboxError::InvalidPreparation);
        }
        let parts = [
            self.limits.resources().memory_bytes(),
            self.runtime_policy.max_total_bytes(),
            self.selected_policy.max_total_bytes() as u64,
            self.selected_policy.max_total_bytes() as u64,
            MAX_WORKSPACE_FILE_READ_BYTES as u64,
            64 * 1024,
            MAX_SETUP_BYTES as u64,
            super::MAX_FRAME_BYTES as u64,
            8192,
        ];
        let mut total = 0_u64;
        for part in parts {
            total = total
                .checked_add(part)
                .ok_or(AdmittedSandboxError::InvalidPreparation)?;
        }
        if total > i64::MAX as u64 {
            return Err(AdmittedSandboxError::InvalidPreparation);
        }
        Ok(total)
    }
}

#[derive(Debug)]
pub enum AdmittedSandboxError {
    InvalidPreparation,
    Admission(PreparationAdmissionError),
    HostMemory(HostMemoryError),
    Runtime(RuntimeManifestError),
    Snapshots(SnapshotPreparationError),
    Execution(SandboxExecutionError),
    CleanupUncertain,
}
impl fmt::Display for AdmittedSandboxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => error.fmt(formatter),
            Self::HostMemory(error) => error.fmt(formatter),
            Self::Execution(error) => error.fmt(formatter),
            Self::InvalidPreparation => {
                formatter.write_str("sandbox preparation intent is invalid")
            }
            Self::Runtime(_) => formatter.write_str("sandbox runtime preparation failed"),
            Self::Snapshots(_) => {
                formatter.write_str("sandbox selected snapshot preparation failed")
            }
            Self::CleanupUncertain => formatter.write_str(
                "sandbox preparation cleanup is uncertain; capacity remains quarantined",
            ),
        }
    }
}
impl Error for AdmittedSandboxError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparationPhase {
    Ready,
    Running,
    CleanupUncertain,
    Released,
}

fn completion_phase(
    result: &Result<OwnedProcessOutput, SandboxExecutionError>,
) -> PreparationPhase {
    if matches!(result, Err(SandboxExecutionError::Process(_))) {
        PreparationPhase::CleanupUncertain
    } else {
        PreparationPhase::Ready
    }
}

/// Owns reviewed payloads before their capacity lease in drop order. Explicit release closes the
/// payloads first; abandoning the object never silently gives budget back while consumers may live.
///
/// ```compile_fail
/// fn log(proposal: &carapana_daemon::AdmittedSandboxProposal<'_, '_>) {
///     let _ = format!("{proposal:?}");
/// }
/// ```
pub struct AdmittedSandboxProposal<'store, 'admission> {
    proposal: Option<SandboxExecutionProposal<'store>>,
    admission: Option<PreparationLease<'admission>>,
    phase: Cell<PreparationPhase>,
    host_memory_policy: HostMemoryAdmissionPolicy,
}
impl<'store, 'admission> AdmittedSandboxProposal<'store, 'admission> {
    pub fn prepare(
        spec: SandboxPreparationSpec<'_>,
        source: &WorkspaceMetadata,
        artifacts: &'store mut ArtifactStore,
        admissions: &'admission mut PreparationAdmissionStore,
        now_ms: i64,
    ) -> Result<Self, AdmittedSandboxError> {
        Self::prepare_inner(spec, source, artifacts, admissions, None, now_ms)
    }

    /// A queue ticket grants capacity order only; source, budgets, memory and later approval are
    /// still independently validated. Recovered waiting metadata never executes this method itself.
    pub fn prepare_queued(
        spec: SandboxPreparationSpec<'_>,
        source: &WorkspaceMetadata,
        artifacts: &'store mut ArtifactStore,
        admissions: &'admission mut PreparationAdmissionStore,
        ticket: &carapana_storage::PreparationQueueTicket,
        now_ms: i64,
    ) -> Result<Self, AdmittedSandboxError> {
        Self::prepare_inner(spec, source, artifacts, admissions, Some(ticket), now_ms)
    }

    fn prepare_inner(
        spec: SandboxPreparationSpec<'_>,
        source: &WorkspaceMetadata,
        artifacts: &'store mut ArtifactStore,
        admissions: &'admission mut PreparationAdmissionStore,
        ticket: Option<&carapana_storage::PreparationQueueTicket>,
        now_ms: i64,
    ) -> Result<Self, AdmittedSandboxError> {
        let required = spec.required_bytes()?;
        source
            .verify_current()
            .map_err(|_| AdmittedSandboxError::InvalidPreparation)?;
        // Must precede runtime verification/freezing and selected file reads/staging/sealing.
        let admission = match ticket {
            Some(ticket) => admissions.claim_queued_preparation(
                ticket,
                required,
                now_ms,
                &source.directory_identity(),
            ),
            None => admissions.reserve_unexported_for_workspace(
                required,
                now_ms,
                &source.directory_identity(),
            ),
        }
        .map_err(AdmittedSandboxError::Admission)?;
        let built = (|| {
            spec.host_memory_policy
                .check(required)
                .map_err(AdmittedSandboxError::HostMemory)?;
            let runtime = VerifiedRuntimeManifest::verify(spec.runtime_files, spec.runtime_policy)
                .and_then(|manifest| manifest.freeze(spec.runtime_policy))
                .map_err(AdmittedSandboxError::Runtime)?;
            let selected = ReservedSelectedSnapshotBundle::prepare(
                artifacts,
                source,
                spec.selected_paths,
                spec.selected_policy,
                now_ms,
            )
            .and_then(|bundle| bundle.seal_for_launch())
            .map_err(AdmittedSandboxError::Snapshots)?;
            let launch = SandboxImageLaunch::new(
                runtime,
                spec.command.executable().to_owned(),
                spec.command
                    .arguments()
                    .map(std::ffi::OsStr::to_owned)
                    .collect(),
                spec.scratch_bytes,
                spec.runtime_policy,
            )
            .and_then(|launch| launch.with_workspace(selected))
            .map_err(|_| AdmittedSandboxError::InvalidPreparation)?;
            SandboxExecutionProposal::new(
                spec.command.workspace().clone(),
                spec.command.task().clone(),
                source,
                launch,
                spec.limits,
            )
            .map_err(AdmittedSandboxError::Execution)
        })();
        match built {
            Ok(proposal) => Ok(Self {
                proposal: Some(proposal),
                admission: Some(admission),
                phase: Cell::new(PreparationPhase::Ready),
                host_memory_policy: spec.host_memory_policy,
            }),
            Err(error) => {
                // No descriptors were exported. Closure locals/payloads have already been dropped.
                admission
                    .release()
                    .map_err(AdmittedSandboxError::Admission)?;
                Err(error)
            }
        }
    }

    pub fn reserved_preparation_bytes(&self) -> u64 {
        self.admission
            .as_ref()
            .expect("live admitted proposal owns its lease")
            .reserved_bytes()
    }
    fn reviewed(&self) -> &SandboxExecutionProposal<'store> {
        self.proposal
            .as_ref()
            .expect("live admitted proposal owns its payloads")
    }
    pub fn program(&self) -> &Path {
        self.reviewed().launch().program()
    }
    pub fn arguments(&self) -> &[std::ffi::OsString] {
        self.reviewed().launch().arguments()
    }
    pub fn scratch_bytes(&self) -> u64 {
        self.reviewed().launch().scratch_bytes()
    }
    pub fn limits(&self) -> SandboxExecutionLimits {
        self.reviewed().limits()
    }
    pub fn host_memory_policy(&self) -> HostMemoryAdmissionPolicy {
        self.host_memory_policy
    }
    pub fn runtime_files(
        &self,
    ) -> impl ExactSizeIterator<
        Item = (
            &Path,
            carapana_sandbox::runtime_manifest::RuntimeFileRole,
            u64,
            &[u8; 32],
        ),
    > {
        self.reviewed().launch().runtime_files().map(|file| {
            (
                file.destination(),
                file.role(),
                file.size_bytes(),
                file.sha256(),
            )
        })
    }
    pub fn selected_files(&self) -> impl Iterator<Item = (&Path, usize, &[u8; 32])> {
        self.reviewed()
            .launch()
            .workspace()
            .into_iter()
            .flat_map(|image| image.selected_files())
    }
    pub fn request_approval(
        &self,
        expires_at: u64,
        now: u64,
    ) -> Result<ProposalApproval<'_, Self>, CommandApprovalError> {
        ProposalApproval::new(self, expires_at, now)
    }

    /// Consume only after dropping approval controllers/tokens. Close payloads BEFORE accounting.
    /// Unknown process/cleanup outcomes refuse release; Drop keeps their capacity charged.
    ///
    /// ```compile_fail
    /// fn early(proposal: carapana_daemon::AdmittedSandboxProposal<'_, '_>) {
    ///     let request = proposal.request_approval(20, 1).unwrap();
    ///     proposal.release();
    ///     let _still_reviewing = request.proposal();
    /// }
    /// ```
    pub fn release(mut self) -> Result<(), AdmittedSandboxError> {
        self.release_ready()
    }

    fn release_ready(&mut self) -> Result<(), AdmittedSandboxError> {
        if self.phase.get() == PreparationPhase::Released {
            return Ok(());
        }
        if self.phase.get() != PreparationPhase::Ready {
            return Err(AdmittedSandboxError::CleanupUncertain);
        }
        self.phase.set(PreparationPhase::CleanupUncertain);
        drop(self.proposal.take());
        if let Some(admission) = self.admission.take() {
            admission
                .release()
                .map_err(AdmittedSandboxError::Admission)?;
        }
        self.phase.set(PreparationPhase::Released);
        Ok(())
    }
}
impl Drop for AdmittedSandboxProposal<'_, '_> {
    fn drop(&mut self) {
        if self.phase.get() == PreparationPhase::Ready {
            // Ready is an independent lifecycle proof (never exported, or cleanup confirmed), not
            // an inference from Drop. Any failed release remains charged by the raw lease's Drop.
            let _ = self.release_ready();
        }
    }
}
impl ApprovalSubject for AdmittedSandboxProposal<'_, '_> {
    fn workspace(&self) -> &WorkspaceId {
        self.reviewed().workspace()
    }
    fn task(&self) -> &TaskId {
        self.reviewed().task()
    }
}
impl<'store> ReviewedSandboxSubject<'store> for AdmittedSandboxProposal<'store, '_> {
    fn execution(&self) -> &SandboxExecutionProposal<'store> {
        self.reviewed()
    }
    fn validate_preparation(&self) -> Result<(), SandboxExecutionError> {
        self.host_memory_policy
            .check(self.limits().resources().memory_bytes())
            .map_err(SandboxExecutionError::HostMemory)
    }
    fn validate_running_memory(&self) -> Result<(), SandboxExecutionError> {
        // The target is already charged. Do not require its full ceiling a second time.
        self.host_memory_policy
            .check(0)
            .map_err(SandboxExecutionError::HostMemory)
    }
}

impl ProcessSupervisor {
    /// The composed internal entry: capacity retained before images, exact token, real cgroup and
    /// verified cleanup. Still requires independent UI-origin/content/secret gates before tool wiring.
    ///
    /// ```compile_fail
    /// fn substitute(supervisor: &mut carapana_daemon::ProcessSupervisor,
    ///     token: carapana_core::command_approval::ApprovedProposal<'_, carapana_daemon::SandboxExecutionProposal<'_>>,
    ///     scope: carapana_daemon::SandboxExecutionScope<'_>,
    ///     group: &mut carapana_sandbox::delegated_cgroup::PreparedCgroup<'_>) {
    ///     supervisor.run_admitted_image_process(token, scope, group, || 1, || false);
    /// }
    /// ```
    pub fn run_admitted_image_process(
        &mut self,
        approval: ApprovedProposal<'_, AdmittedSandboxProposal<'_, '_>>,
        scope: SandboxExecutionScope<'_>,
        group: &mut PreparedCgroup<'_>,
        clock: impl FnMut() -> u64,
        is_cancelled: impl FnMut() -> bool,
    ) -> Result<OwnedProcessOutput, AdmittedSandboxError> {
        let admitted = approval.proposal();
        if admitted.phase.get() != PreparationPhase::Ready {
            return Err(AdmittedSandboxError::CleanupUncertain);
        }
        // Payloads already exist; reserve room for the still-unstarted cgroup, not a second copy
        // of the whole preparation. No host or target has started when this check rejects.
        admitted
            .host_memory_policy
            .check(admitted.limits().resources().memory_bytes())
            .map_err(AdmittedSandboxError::HostMemory)?;
        admitted
            .admission
            .as_ref()
            .ok_or(AdmittedSandboxError::CleanupUncertain)?
            .mark_exported()
            .map_err(AdmittedSandboxError::Admission)?;
        // A panic, reentrant attempt or unknown process failure must never enable release.
        admitted.phase.set(PreparationPhase::Running);
        let result = self.run_reviewed_image_process(&approval, scope, group, clock, is_cancelled);
        if completion_phase(&result) == PreparationPhase::Ready {
            // The supervisor returned only after cgroup termination and guardian reap, or before
            // any consumer existed. Persist that proof before allowing Ready/reuse/recovery.
            admitted.phase.set(PreparationPhase::CleanupUncertain);
            admitted
                .admission
                .as_ref()
                .ok_or(AdmittedSandboxError::CleanupUncertain)?
                .mark_cleanup_confirmed()
                .map_err(AdmittedSandboxError::Admission)?;
        }
        admitted.phase.set(completion_phase(&result));
        result.map_err(AdmittedSandboxError::Execution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_not_enable_release_after_unknown_process_or_cleanup_results() {
        for error in [
            ProcessError::ResourceCleanupFailed,
            ProcessError::HostProtocol,
            ProcessError::HostTimedOut,
            ProcessError::ShutdownTimedOut,
        ] {
            assert!(
                completion_phase(&Err(SandboxExecutionError::Process(error)))
                    == PreparationPhase::CleanupUncertain
            );
        }
        for error in [
            SandboxExecutionError::Approval(CommandApprovalError::Expired),
            SandboxExecutionError::Approval(CommandApprovalError::Invalidated),
            SandboxExecutionError::StagingChanged,
            SandboxExecutionError::HostMemory(HostMemoryError::PressureExceeded),
        ] {
            assert!(completion_phase(&Err(error)) == PreparationPhase::Ready);
        }
    }
}
