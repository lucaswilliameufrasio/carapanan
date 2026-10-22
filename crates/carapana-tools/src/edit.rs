//! A local exact-approved edit over the existing snapshot/atomic-write primitives. No runtime wiring.

use crate::{TextDiffPreview, TextDiffPreviewError, create_snapshot_diff_preview};
use carapana_core::{
    command_approval::{ApprovalSubject, ApprovedProposal, CommandApprovalError, ProposalApproval},
    trust::{TaskId, WorkspaceId},
};
use carapana_storage::{WorkspaceFileSnapshot, WorkspaceFileWriteError, WorkspaceMetadata};
use std::path::Path;

/// Immutable candidate and original snapshot. No Debug, Clone, serialization, or automatic sharing.
///
/// ```compile_fail
/// fn log(edit: &carapana_tools::edit::WorkspaceEditProposal) {
///     let _unsafe_log = format!("{edit:?}");
/// }
/// ```
pub struct WorkspaceEditProposal {
    workspace: WorkspaceId,
    task: TaskId,
    snapshot: WorkspaceFileSnapshot,
    replacement: String,
    preview: TextDiffPreview,
}

impl WorkspaceEditProposal {
    pub fn new(
        workspace: WorkspaceId,
        task: TaskId,
        snapshot: WorkspaceFileSnapshot,
        replacement: String,
    ) -> Result<Self, TextDiffPreviewError> {
        let preview = create_snapshot_diff_preview(&snapshot, &replacement)?
            .preview()
            .clone();
        Ok(Self {
            workspace,
            task,
            snapshot,
            replacement,
            preview,
        })
    }

    /// Untrusted source path; escape before display.
    pub fn relative_path(&self) -> &Path {
        self.snapshot.relative_path()
    }
    /// Untrusted local preview; sanitization does not classify secrets or make this an applicable patch.
    pub fn preview(&self) -> &TextDiffPreview {
        &self.preview
    }
    pub fn request_approval(
        &self,
        expires_at: u64,
        now: u64,
    ) -> Result<ProposalApproval<'_, Self>, CommandApprovalError> {
        ProposalApproval::new(self, expires_at, now)
    }
}

impl ApprovalSubject for WorkspaceEditProposal {
    fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }
    fn task(&self) -> &TaskId {
        &self.task
    }
}

#[derive(Debug)]
pub enum WorkspaceEditError {
    Approval(CommandApprovalError),
    Write(WorkspaceFileWriteError),
}

impl std::fmt::Display for WorkspaceEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Approval(_) => formatter.write_str("workspace edit approval is no longer valid"),
            Self::Write(_) => formatter
                .write_str("workspace edit could not be safely committed; review before retrying"),
        }
    }
}
impl std::error::Error for WorkspaceEditError {}

/// Consume one exact token, never a path/replacement supplied after review.
/// Caller must authenticate operator decisions and independently enforce secret/concurrency policy.
/// The storage writer revalidates snapshot identity/content; it is not a filesystem-wide CAS.
/// A failed or uncertain effect consumes the token too, requiring a fresh operator decision.
///
/// ```compile_fail
/// fn substitute(token: carapana_core::command_approval::ApprovedCommand<'_>,
///     workspace: &carapana_storage::WorkspaceMetadata,
///     id: &carapana_core::trust::WorkspaceId, task: &carapana_core::trust::TaskId) {
///     carapana_tools::edit::apply_approved_workspace_edit(token, workspace, id, task, 1);
/// }
/// ```
///
/// ```compile_fail
/// fn twice(token: carapana_core::command_approval::ApprovedProposal<'_, carapana_tools::edit::WorkspaceEditProposal>,
///     workspace: &carapana_storage::WorkspaceMetadata,
///     id: &carapana_core::trust::WorkspaceId, task: &carapana_core::trust::TaskId) {
///     carapana_tools::edit::apply_approved_workspace_edit(token, workspace, id, task, 1);
///     carapana_tools::edit::apply_approved_workspace_edit(token, workspace, id, task, 1);
/// }
/// ```
pub fn apply_approved_workspace_edit(
    approval: ApprovedProposal<'_, WorkspaceEditProposal>,
    workspace: &WorkspaceMetadata,
    workspace_id: &WorkspaceId,
    task: &TaskId,
    now: u64,
) -> Result<(), WorkspaceEditError> {
    approval
        .validate_for_effect(workspace_id, task, now)
        .map_err(WorkspaceEditError::Approval)?;
    let proposal = approval.proposal();
    workspace
        .replace_file_atomically(&proposal.snapshot, proposal.replacement.as_bytes())
        .map_err(WorkspaceEditError::Write)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, PermissionsExt},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture {
        root: PathBuf,
        metadata: WorkspaceMetadata,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "carapana-edit-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            fs::write(root.join("source.txt"), b"before\n").unwrap();
            fs::set_permissions(root.join("source.txt"), fs::Permissions::from_mode(0o600))
                .unwrap();
            let metadata = WorkspaceMetadata::capture(&root).unwrap();
            Self { root, metadata }
        }
        fn proposal(&self, after: &str) -> WorkspaceEditProposal {
            WorkspaceEditProposal::new(
                WorkspaceId::new("w").unwrap(),
                TaskId::new("t").unwrap(),
                self.metadata.read_file_snapshot("source.txt").unwrap(),
                after.to_owned(),
            )
            .unwrap()
        }
        fn bytes(&self) -> Vec<u8> {
            fs::read(self.root.join("source.txt")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn should_apply_only_the_candidate_bound_to_the_reviewed_snapshot_and_request() {
        let fixture = Fixture::new();
        let proposal = fixture.proposal("after\n");
        let other = fixture.proposal("not approved\n");
        assert_eq!(proposal.relative_path(), Path::new("source.txt"));
        assert!(proposal.preview().unified_diff.contains("+after"));
        let mut request = proposal.request_approval(20, 10).unwrap();
        let other_request = other.request_approval(20, 10).unwrap();
        assert!(matches!(
            request.approve_by_operator(
                &other_request.id(),
                proposal.workspace(),
                proposal.task(),
                11
            ),
            Err(CommandApprovalError::WrongRequest)
        ));
        let token = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        apply_approved_workspace_edit(
            token,
            &fixture.metadata,
            proposal.workspace(),
            proposal.task(),
            12,
        )
        .unwrap();
        assert_eq!(fixture.bytes(), b"after\n");
        assert!(matches!(
            request.approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 12),
            Err(CommandApprovalError::NotPending)
        ));
    }

    #[test]
    fn should_preserve_concurrent_work_and_require_a_new_decision_after_conflict() {
        let fixture = Fixture::new();
        let proposal = fixture.proposal("candidate\n");
        let mut request = proposal.request_approval(20, 10).unwrap();
        let token = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        fs::write(fixture.root.join("source.txt"), b"external edit\n").unwrap();
        assert!(matches!(
            apply_approved_workspace_edit(
                token,
                &fixture.metadata,
                proposal.workspace(),
                proposal.task(),
                12
            ),
            Err(WorkspaceEditError::Write(
                WorkspaceFileWriteError::Conflict(_)
            ))
        ));
        assert_eq!(fixture.bytes(), b"external edit\n");
        assert!(matches!(
            request.approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 12),
            Err(CommandApprovalError::NotPending)
        ));
    }

    #[test]
    fn should_reject_expired_revoked_and_wrong_scope_approvals_without_writing() {
        for case in 0..4 {
            let fixture = Fixture::new();
            let proposal = fixture.proposal("candidate\n");
            let mut request = proposal.request_approval(20, 10).unwrap();
            let token = request
                .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
                .unwrap();
            let other_workspace = WorkspaceId::new("other").unwrap();
            let other_task = TaskId::new("other").unwrap();
            let (workspace, task, now) = match case {
                0 => (proposal.workspace(), proposal.task(), 20),
                1 => {
                    request.invalidate();
                    (proposal.workspace(), proposal.task(), 12)
                }
                2 => (&other_workspace, proposal.task(), 12),
                _ => (proposal.workspace(), &other_task, 12),
            };
            assert!(matches!(
                apply_approved_workspace_edit(token, &fixture.metadata, workspace, task, now),
                Err(WorkspaceEditError::Approval(_))
            ));
            assert_eq!(fixture.bytes(), b"before\n");
        }
    }

    #[test]
    fn should_reject_another_filesystem_workspace_even_when_logical_ids_match() {
        let first = Fixture::new();
        let other = Fixture::new();
        let proposal = first.proposal("candidate\n");
        let mut request = proposal.request_approval(20, 10).unwrap();
        let token = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        assert!(matches!(
            apply_approved_workspace_edit(
                token,
                &other.metadata,
                proposal.workspace(),
                proposal.task(),
                12
            ),
            Err(WorkspaceEditError::Write(
                WorkspaceFileWriteError::SnapshotFromDifferentWorkspace
            ))
        ));
        assert_eq!(first.bytes(), b"before\n");
        assert_eq!(other.bytes(), b"before\n");
    }

    #[test]
    fn should_preserve_original_bytes_when_the_request_controller_is_discarded() {
        let fixture = Fixture::new();
        let proposal = fixture.proposal("candidate\n");
        let mut request = proposal.request_approval(20, 10).unwrap();
        let token = request
            .approve_by_operator(&request.id(), proposal.workspace(), proposal.task(), 11)
            .unwrap();
        drop(request);
        assert!(matches!(
            apply_approved_workspace_edit(
                token,
                &fixture.metadata,
                proposal.workspace(),
                proposal.task(),
                12
            ),
            Err(WorkspaceEditError::Approval(
                CommandApprovalError::Invalidated
            ))
        ));
        assert_eq!(fixture.bytes(), b"before\n");
    }

    #[test]
    fn should_reject_binary_and_over_limit_candidates_before_requesting_approval() {
        let fixture = Fixture::new();
        for (candidate, expected) in [
            (
                "binary\0candidate".to_owned(),
                TextDiffPreviewError::BinaryInput,
            ),
            (
                "x".repeat(crate::MAX_DIFF_SOURCE_BYTES + 1),
                TextDiffPreviewError::SourceTooLarge,
            ),
        ] {
            let snapshot = fixture.metadata.read_file_snapshot("source.txt").unwrap();
            assert!(
                matches!(WorkspaceEditProposal::new(WorkspaceId::new("w").unwrap(), TaskId::new("t").unwrap(), snapshot, candidate),
                Err(error) if error == expected)
            );
        }
        assert_eq!(fixture.bytes(), b"before\n");
    }
}
