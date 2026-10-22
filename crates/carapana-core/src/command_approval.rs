//! Exact, ephemeral proposal approval contracts, with compatibility aliases for commands.
//! No paths are opened and no effects are executed here.
//!
//! An approval token is only one gate. A future effect boundary must take it by value, validate it
//! immediately before the effect, and independently enforce sandbox/content/resource/concurrency policy.

use crate::trust::{TaskId, WorkspaceId};
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_ARGUMENT_COUNT: usize = 128;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandProposalError {
    InvalidExecutable,
    InvalidWorkingDirectory,
    TooManyArguments,
    ArgumentsTooLarge,
    ArgumentContainsNul,
}

/// An immutable, bounded command description; this is not a safe-to-execute command.
/// Payloads intentionally implement neither `Debug`, `Clone`, nor serialization.
pub struct CommandProposal {
    workspace: WorkspaceId,
    task: TaskId,
    executable: PathBuf,
    arguments: Vec<OsString>,
    relative_working_directory: PathBuf,
}

impl CommandProposal {
    pub fn new(
        workspace: WorkspaceId,
        task: TaskId,
        executable: PathBuf,
        arguments: Vec<OsString>,
        relative_working_directory: PathBuf,
    ) -> Result<Self, CommandProposalError> {
        if !executable.is_absolute()
            || executable.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
            || executable.as_os_str().as_encoded_bytes().contains(&0)
            || executable.file_name().is_none()
            || executable.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::CurDir | Component::Prefix(_)
                )
            })
        {
            return Err(CommandProposalError::InvalidExecutable);
        }
        if relative_working_directory
            .as_os_str()
            .as_encoded_bytes()
            .len()
            > MAX_PATH_BYTES
            || relative_working_directory
                .as_os_str()
                .as_encoded_bytes()
                .contains(&0)
            || relative_working_directory
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(CommandProposalError::InvalidWorkingDirectory);
        }
        if arguments.len() > MAX_ARGUMENT_COUNT {
            return Err(CommandProposalError::TooManyArguments);
        }
        let mut argument_bytes = 0_usize;
        for argument in &arguments {
            let bytes = argument.as_encoded_bytes();
            if bytes.contains(&0) {
                return Err(CommandProposalError::ArgumentContainsNul);
            }
            argument_bytes = argument_bytes
                .checked_add(bytes.len())
                .ok_or(CommandProposalError::ArgumentsTooLarge)?;
            if argument_bytes > MAX_ARGUMENT_BYTES {
                return Err(CommandProposalError::ArgumentsTooLarge);
            }
        }
        Ok(Self {
            workspace,
            task,
            executable,
            arguments,
            relative_working_directory,
        })
    }

    pub fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }
    pub fn task(&self) -> &TaskId {
        &self.task
    }
    /// Untrusted path for review; not canonicalized or authorized for access.
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    /// Untrusted arguments: escape before display and apply separate secret gates.
    pub fn arguments(&self) -> impl ExactSizeIterator<Item = &OsStr> {
        self.arguments.iter().map(OsString::as_os_str)
    }
    /// An empty relative cwd represents the workspace root, not the caller's current directory.
    pub fn relative_working_directory(&self) -> &Path {
        &self.relative_working_directory
    }
}

/// Trusted application proposal types implement this with immutable workspace/task and payload fields.
/// The trait models scope only: it cannot prove immutability of arbitrary external implementations.
pub trait ApprovalSubject {
    fn workspace(&self) -> &WorkspaceId;
    fn task(&self) -> &TaskId;
}

impl ApprovalSubject for CommandProposal {
    fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }
    fn task(&self) -> &TaskId {
        &self.task
    }
}

pub type CommandApproval<'proposal> = ProposalApproval<'proposal, CommandProposal>;
pub type ApprovedCommand<'proposal> = ApprovedProposal<'proposal, CommandProposal>;

/// In-memory identity. It cannot be forged from a numeric/string ID or survive a restart.
#[derive(Clone)]
pub struct CommandApprovalId(Arc<()>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandApprovalState {
    Pending,
    Approved,
    Denied,
    Invalidated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandApprovalError {
    ExpiryNotInFuture,
    Expired,
    WrongRequest,
    WrongWorkspace,
    WrongTask,
    NotPending,
    Invalidated,
}

/// A single request for one immutable proposal. Callers must authenticate operator event origin.
pub struct ProposalApproval<'proposal, P: ApprovalSubject> {
    proposal: &'proposal P,
    id: CommandApprovalId,
    state: CommandApprovalState,
    expires_at: u64,
    revoked: Arc<AtomicBool>,
}

impl<'proposal, P: ApprovalSubject> ProposalApproval<'proposal, P> {
    /// `now` and `expires_at` must use the same caller-defined monotonic clock.
    pub fn new(
        proposal: &'proposal P,
        expires_at: u64,
        now: u64,
    ) -> Result<Self, CommandApprovalError> {
        if expires_at <= now {
            return Err(CommandApprovalError::ExpiryNotInFuture);
        }
        Ok(Self {
            proposal,
            id: CommandApprovalId(Arc::new(())),
            state: CommandApprovalState::Pending,
            expires_at,
            revoked: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn id(&self) -> CommandApprovalId {
        self.id.clone()
    }
    pub fn state(&self) -> CommandApprovalState {
        self.state
    }
    pub fn proposal(&self) -> &P {
        self.proposal
    }

    /// This method records a direct operator decision; it does not authenticate a UI or model event.
    pub fn approve_by_operator(
        &mut self,
        id: &CommandApprovalId,
        workspace: &WorkspaceId,
        task: &TaskId,
        now: u64,
    ) -> Result<ApprovedProposal<'proposal, P>, CommandApprovalError> {
        self.validate_decision(id, workspace, task, now)?;
        self.state = CommandApprovalState::Approved;
        Ok(ApprovedProposal {
            proposal: self.proposal,
            expires_at: self.expires_at,
            revoked: Arc::clone(&self.revoked),
        })
    }

    pub fn deny_by_operator(
        &mut self,
        id: &CommandApprovalId,
        workspace: &WorkspaceId,
        task: &TaskId,
        now: u64,
    ) -> Result<(), CommandApprovalError> {
        self.validate_decision(id, workspace, task, now)?;
        self.state = CommandApprovalState::Denied;
        self.revoked.store(true, Ordering::Release);
        Ok(())
    }

    /// Revoke pending and already-issued approvals after intervention or scope changes.
    pub fn invalidate(&mut self) {
        self.revoked.store(true, Ordering::Release);
        if matches!(
            self.state,
            CommandApprovalState::Pending | CommandApprovalState::Approved
        ) {
            self.state = CommandApprovalState::Invalidated;
        }
    }

    fn validate_decision(
        &self,
        id: &CommandApprovalId,
        workspace: &WorkspaceId,
        task: &TaskId,
        now: u64,
    ) -> Result<(), CommandApprovalError> {
        if !Arc::ptr_eq(&self.id.0, &id.0) {
            return Err(CommandApprovalError::WrongRequest);
        }
        if self.revoked.load(Ordering::Acquire) && self.state == CommandApprovalState::Invalidated {
            return Err(CommandApprovalError::Invalidated);
        }
        if self.state != CommandApprovalState::Pending {
            return Err(CommandApprovalError::NotPending);
        }
        validate_scope(self.proposal, self.expires_at, workspace, task, now)
    }
}

impl<P: ApprovalSubject> Drop for ProposalApproval<'_, P> {
    fn drop(&mut self) {
        self.revoked.store(true, Ordering::Release);
    }
}

/// Non-cloneable, borrowed, one-shot approval token. This is not a process-execution capability.
/// A future executor must consume this token by value and independently enforce all remaining gates.
/// The request controller must remain alive: dropping it revokes all issued tokens.
///
/// ```compile_fail
/// fn duplicate(token: carapana_core::command_approval::ApprovedCommand<'_>) {
///     let _duplicate = token.clone();
/// }
/// ```
#[must_use]
pub struct ApprovedProposal<'proposal, P: ApprovalSubject> {
    proposal: &'proposal P,
    expires_at: u64,
    revoked: Arc<AtomicBool>,
}

impl<P: ApprovalSubject> ApprovedProposal<'_, P> {
    pub fn proposal(&self) -> &P {
        self.proposal
    }

    pub fn validate_for_effect(
        &self,
        workspace: &WorkspaceId,
        task: &TaskId,
        now: u64,
    ) -> Result<(), CommandApprovalError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(CommandApprovalError::Invalidated);
        }
        validate_scope(self.proposal, self.expires_at, workspace, task, now)
    }
}

fn validate_scope(
    proposal: &impl ApprovalSubject,
    expires_at: u64,
    workspace: &WorkspaceId,
    task: &TaskId,
    now: u64,
) -> Result<(), CommandApprovalError> {
    if workspace != proposal.workspace() {
        return Err(CommandApprovalError::WrongWorkspace);
    }
    if task != proposal.task() {
        return Err(CommandApprovalError::WrongTask);
    }
    if now >= expires_at {
        return Err(CommandApprovalError::Expired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(argument: &str) -> CommandProposal {
        CommandProposal::new(
            WorkspaceId::new("workspace-a").unwrap(),
            TaskId::new("task-a").unwrap(),
            PathBuf::from("/usr/bin/example"),
            vec![OsString::from(argument)],
            PathBuf::from("src"),
        )
        .unwrap()
    }

    #[test]
    fn should_bind_a_single_approval_to_the_exact_immutable_proposal() {
        let proposal = proposal("synthetic argument");
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let id = request.id();
        let approved = request
            .approve_by_operator(&id, proposal.workspace(), proposal.task(), 11)
            .unwrap();
        assert_eq!(request.state(), CommandApprovalState::Approved);
        assert!(std::ptr::eq(approved.proposal(), &proposal));
        assert_eq!(
            approved.proposal().arguments().next(),
            Some(OsStr::new("synthetic argument"))
        );
        assert!(
            approved
                .validate_for_effect(proposal.workspace(), proposal.task(), 19)
                .is_ok()
        );
        assert!(matches!(
            request.approve_by_operator(&id, proposal.workspace(), proposal.task(), 12),
            Err(CommandApprovalError::NotPending)
        ));
        assert_eq!(
            approved.validate_for_effect(proposal.workspace(), proposal.task(), 20),
            Err(CommandApprovalError::Expired)
        );
    }

    #[test]
    fn should_refuse_another_request_even_for_the_same_workspace_and_task() {
        let first = proposal("first");
        let second = proposal("second");
        let mut request = CommandApproval::new(&first, 20, 10).unwrap();
        let other_request = CommandApproval::new(&second, 20, 10).unwrap();
        assert!(matches!(
            request.approve_by_operator(&other_request.id(), first.workspace(), first.task(), 11),
            Err(CommandApprovalError::WrongRequest)
        ));
        assert_eq!(request.state(), CommandApprovalState::Pending);
    }

    #[test]
    fn should_reject_scope_mismatch_without_consuming_a_pending_request() {
        let proposal = proposal("first");
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let id = request.id();
        assert!(matches!(
            request.approve_by_operator(
                &id,
                &WorkspaceId::new("other").unwrap(),
                proposal.task(),
                11
            ),
            Err(CommandApprovalError::WrongWorkspace)
        ));
        assert!(matches!(
            request.approve_by_operator(
                &id,
                proposal.workspace(),
                &TaskId::new("other").unwrap(),
                11
            ),
            Err(CommandApprovalError::WrongTask)
        ));
        assert_eq!(request.state(), CommandApprovalState::Pending);
        assert!(
            request
                .approve_by_operator(&id, proposal.workspace(), proposal.task(), 11)
                .is_ok()
        );
    }

    #[test]
    fn should_revoke_tokens_already_issued_before_intervention() {
        let proposal = proposal("first");
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let approved = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        request.invalidate();
        assert_eq!(request.state(), CommandApprovalState::Invalidated);
        assert_eq!(
            approved.validate_for_effect(proposal.workspace(), proposal.task(), 12),
            Err(CommandApprovalError::Invalidated)
        );
    }

    #[test]
    fn should_revoke_issued_tokens_when_the_request_controller_is_dropped() {
        let proposal = proposal("first");
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let approved = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        drop(request);
        assert_eq!(
            approved.validate_for_effect(proposal.workspace(), proposal.task(), 12),
            Err(CommandApprovalError::Invalidated)
        );
    }

    #[test]
    fn should_recheck_workspace_and_task_at_the_future_effect_boundary() {
        let proposal = proposal("first");
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let approved = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        assert_eq!(
            approved.validate_for_effect(&WorkspaceId::new("other").unwrap(), proposal.task(), 12),
            Err(CommandApprovalError::WrongWorkspace)
        );
        assert_eq!(
            approved.validate_for_effect(proposal.workspace(), &TaskId::new("other").unwrap(), 12),
            Err(CommandApprovalError::WrongTask)
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_preserve_non_utf8_arguments_without_rendering_or_logging_them() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let argument = OsString::from_vec(vec![b'a', 0xff, b'b']);
        let proposal = CommandProposal::new(
            WorkspaceId::new("w").unwrap(),
            TaskId::new("t").unwrap(),
            PathBuf::from("/usr/bin/tool"),
            vec![argument],
            PathBuf::new(),
        )
        .unwrap();
        assert_eq!(
            proposal.arguments().next().unwrap().as_bytes(),
            &[b'a', 0xff, b'b']
        );
        assert!(proposal.relative_working_directory().as_os_str().is_empty());
        assert_eq!(proposal.executable(), Path::new("/usr/bin/tool"));
    }

    #[test]
    fn should_keep_denied_invalidated_and_expired_requests_closed() {
        let proposal = proposal("first");
        assert!(matches!(
            CommandApproval::new(&proposal, 10, 10),
            Err(CommandApprovalError::ExpiryNotInFuture)
        ));
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        let id = request.id();
        request
            .deny_by_operator(&id, proposal.workspace(), proposal.task(), 11)
            .unwrap();
        assert_eq!(request.state(), CommandApprovalState::Denied);
        assert!(matches!(
            request.approve_by_operator(&id, proposal.workspace(), proposal.task(), 11),
            Err(CommandApprovalError::NotPending)
        ));
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        request.invalidate();
        assert!(matches!(
            request.approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11),
            Err(CommandApprovalError::Invalidated)
        ));
        let mut request = CommandApproval::new(&proposal, 20, 10).unwrap();
        assert!(matches!(
            request.approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 20),
            Err(CommandApprovalError::Expired)
        ));
    }

    #[test]
    fn should_reject_unsafe_structure_before_creating_a_proposal() {
        for executable in ["relative", "/usr/../bin/tool", "/", "/usr/bin/to\0ol"] {
            assert!(matches!(
                CommandProposal::new(
                    WorkspaceId::new("w").unwrap(),
                    TaskId::new("t").unwrap(),
                    PathBuf::from(executable),
                    vec![],
                    PathBuf::new()
                ),
                Err(CommandProposalError::InvalidExecutable)
            ));
        }
        for cwd in ["/outside", "../outside", "src/../../outside", "src\0dir"] {
            assert!(matches!(
                CommandProposal::new(
                    WorkspaceId::new("w").unwrap(),
                    TaskId::new("t").unwrap(),
                    PathBuf::from("/usr/bin/tool"),
                    vec![],
                    PathBuf::from(cwd)
                ),
                Err(CommandProposalError::InvalidWorkingDirectory)
            ));
        }
        assert!(matches!(
            CommandProposal::new(
                WorkspaceId::new("w").unwrap(),
                TaskId::new("t").unwrap(),
                PathBuf::from("/usr/bin/tool"),
                vec![OsString::from("x"); MAX_ARGUMENT_COUNT + 1],
                PathBuf::new()
            ),
            Err(CommandProposalError::TooManyArguments)
        ));
        assert!(matches!(
            CommandProposal::new(
                WorkspaceId::new("w").unwrap(),
                TaskId::new("t").unwrap(),
                PathBuf::from("/usr/bin/tool"),
                vec![OsString::from("x".repeat(MAX_ARGUMENT_BYTES + 1))],
                PathBuf::new()
            ),
            Err(CommandProposalError::ArgumentsTooLarge)
        ));
        assert!(matches!(
            CommandProposal::new(
                WorkspaceId::new("w").unwrap(),
                TaskId::new("t").unwrap(),
                PathBuf::from("/usr/bin/tool"),
                vec![OsString::from("x\0y")],
                PathBuf::new()
            ),
            Err(CommandProposalError::ArgumentContainsNul)
        ));
    }
}
