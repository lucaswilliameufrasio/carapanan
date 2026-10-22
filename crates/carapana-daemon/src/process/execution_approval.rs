//! Exact local execution approval over the composed sandbox. No UI/provider/tool activation.

use super::capture::{AdmissionCheck, ScopedLaunchGate};
use super::*;
use carapana_core::{
    command_approval::{ApprovalSubject, ApprovedProposal, CommandApprovalError, ProposalApproval},
    trust::{TaskId, WorkspaceId},
};
use carapana_sandbox::{
    delegated_cgroup::{CgroupLimits, PreparedCgroup},
    image_launcher::SandboxImageLaunch,
};
use carapana_storage::{WorkspaceDirectoryIdentity, WorkspaceMetadata};

#[derive(Clone, Copy)]
pub struct SandboxExecutionLimits {
    capture: ProcessCaptureSettings,
    resources: CgroupLimits,
}

impl SandboxExecutionLimits {
    pub fn new(
        capture: ProcessCaptureSettings,
        resources: CgroupLimits,
    ) -> Result<Self, SandboxExecutionError> {
        capture.validate().map_err(SandboxExecutionError::Process)?;
        Ok(Self { capture, resources })
    }
    pub fn capture(&self) -> ProcessCaptureSettings {
        self.capture
    }
    pub fn resources(&self) -> CgroupLimits {
        self.resources
    }
}

#[derive(Debug)]
pub enum SandboxExecutionError {
    Approval(CommandApprovalError),
    MissingReservedWorkspace,
    PhysicalWorkspaceMismatch,
    StagingChanged,
    LimitMismatch,
    Process(ProcessError),
}

impl fmt::Display for SandboxExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Approval(_) => "sandbox approval is no longer valid",
            Self::MissingReservedWorkspace => {
                "sandbox execution requires reserved selected snapshots"
            }
            Self::PhysicalWorkspaceMismatch => {
                "sandbox source workspace identity does not match the review"
            }
            Self::StagingChanged => "selected staging changed after review",
            Self::LimitMismatch => "sandbox limits do not match the review",
            Self::Process(ProcessError::ResourceCleanupFailed) => {
                "sandbox cleanup is uncertain; retain resources and review before retrying"
            }
            Self::Process(_) => {
                "sandbox process failed; inspect its cleanup status before retrying"
            }
        })
    }
}
impl Error for SandboxExecutionError {}

/// Immutable command/runtime/snapshot/budget proposal. No Debug, Clone or serialization.
///
/// ```compile_fail
/// fn log(proposal: &carapana_daemon::SandboxExecutionProposal<'_>) {
///     let _ = format!("{proposal:?}");
/// }
/// ```
pub struct SandboxExecutionProposal<'store> {
    workspace: WorkspaceId,
    task: TaskId,
    source_identity: WorkspaceDirectoryIdentity,
    launch: SandboxImageLaunch<'store>,
    limits: SandboxExecutionLimits,
}

impl<'store> SandboxExecutionProposal<'store> {
    pub fn new(
        workspace: WorkspaceId,
        task: TaskId,
        source: &WorkspaceMetadata,
        launch: SandboxImageLaunch<'store>,
        limits: SandboxExecutionLimits,
    ) -> Result<Self, SandboxExecutionError> {
        let selected = launch
            .workspace()
            .ok_or(SandboxExecutionError::MissingReservedWorkspace)?;
        if selected.source_identity() != &source.directory_identity()
            || source.verify_current().is_err()
        {
            return Err(SandboxExecutionError::PhysicalWorkspaceMismatch);
        }
        selected
            .verify()
            .map_err(|_| SandboxExecutionError::StagingChanged)?;
        limits
            .capture
            .validate()
            .map_err(SandboxExecutionError::Process)?;
        Ok(Self {
            workspace,
            task,
            source_identity: source.directory_identity(),
            launch,
            limits,
        })
    }
    pub fn launch(&self) -> &SandboxImageLaunch<'store> {
        &self.launch
    }
    pub fn limits(&self) -> SandboxExecutionLimits {
        self.limits
    }
    pub fn request_approval(
        &self,
        expires_at: u64,
        now: u64,
    ) -> Result<ProposalApproval<'_, Self>, CommandApprovalError> {
        ProposalApproval::new(self, expires_at, now)
    }
}

impl ApprovalSubject for SandboxExecutionProposal<'_> {
    fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }
    fn task(&self) -> &TaskId {
        &self.task
    }
}

/// Scope supplied by the trusted controller, not proof of authenticated UI/model event origin.
pub struct SandboxExecutionScope<'scope> {
    pub session_id: &'scope str,
    pub workspace_id: &'scope WorkspaceId,
    pub task: &'scope TaskId,
    pub source: &'scope WorkspaceMetadata,
}

impl ProcessSupervisor {
    /// Consume an exact token, never replacement argv/runtime/snapshots/budgets supplied after review.
    /// `clock` must be the trusted monotonic clock used by the approval controller, not model input.
    /// Revocation/expiry while running terminates the scope. Cleanup uncertainty takes precedence over
    /// the approval error: caller must inspect it and retain resources, never silently retry.
    /// This does not add secret classification, UI authentication or global preparation admission.
    ///
    /// ```compile_fail
    /// fn substitute(supervisor: &mut carapana_daemon::ProcessSupervisor,
    ///     token: carapana_core::command_approval::ApprovedCommand<'_>,
    ///     scope: carapana_daemon::SandboxExecutionScope<'_>,
    ///     group: &mut carapana_sandbox::delegated_cgroup::PreparedCgroup<'_>) {
    ///     supervisor.run_approved_image_process(token, scope, group, || 1, || false);
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// fn twice(supervisor: &mut carapana_daemon::ProcessSupervisor,
    ///     token: carapana_core::command_approval::ApprovedProposal<'_, carapana_daemon::SandboxExecutionProposal<'_>>,
    ///     scope: carapana_daemon::SandboxExecutionScope<'_>,
    ///     group: &mut carapana_sandbox::delegated_cgroup::PreparedCgroup<'_>) {
    ///     supervisor.run_approved_image_process(token, scope, group, || 1, || false);
    ///     let _again = token;
    /// }
    /// ```
    pub fn run_approved_image_process(
        &mut self,
        approval: ApprovedProposal<'_, SandboxExecutionProposal<'_>>,
        scope: SandboxExecutionScope<'_>,
        group: &mut PreparedCgroup<'_>,
        mut clock: impl FnMut() -> u64,
        is_cancelled: impl FnMut() -> bool,
    ) -> Result<OwnedProcessOutput, SandboxExecutionError> {
        let proposal = approval.proposal();
        let actual_limits = group.limits();
        let mut validation_error = None;
        let result = {
            let mut validate = |phase| {
                let check = (|| {
                    approval
                        .validate_for_effect(scope.workspace_id, scope.task, clock())
                        .map_err(SandboxExecutionError::Approval)?;
                    if let AdmissionCheck::BeforeEffect = phase {
                        if actual_limits != proposal.limits.resources {
                            return Err(SandboxExecutionError::LimitMismatch);
                        }
                        if scope.source.directory_identity() != proposal.source_identity
                            || scope.source.verify_current().is_err()
                        {
                            return Err(SandboxExecutionError::PhysicalWorkspaceMismatch);
                        }
                        proposal
                            .launch
                            .workspace()
                            .ok_or(SandboxExecutionError::MissingReservedWorkspace)?
                            .verify()
                            .map_err(|_| SandboxExecutionError::StagingChanged)?;
                        approval
                            .validate_for_effect(scope.workspace_id, scope.task, clock())
                            .map_err(SandboxExecutionError::Approval)?;
                    }
                    Ok(())
                })();
                if let Err(error) = check {
                    validation_error = Some(error);
                    return Err(ProcessError::ExecutionApprovalInvalid);
                }
                Ok(())
            };
            match validate(AdmissionCheck::BeforeEffect) {
                Err(error) => Err(error),
                Ok(()) => {
                    let spec = ProcessSpec::new(
                        &self.host_executable,
                        vec!["__carapana_internal_image_launcher_v1".into()],
                        "/",
                    );
                    self.run_resource_scoped_inner(
                        scope.session_id,
                        spec,
                        proposal.limits.capture,
                        group,
                        ScopedLaunchGate {
                            image: Some(&proposal.launch),
                            validate: Some(&mut validate),
                        },
                        is_cancelled,
                    )
                }
            }
        };
        match result {
            Err(ProcessError::ExecutionApprovalInvalid) => Err(validation_error.unwrap_or(
                SandboxExecutionError::Process(ProcessError::ExecutionApprovalInvalid),
            )),
            result => result.map_err(SandboxExecutionError::Process),
        }
    }
}
