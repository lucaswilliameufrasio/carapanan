//! Explicit workspace trust and short-lived authorization grants.
//!
//! These types model decisions only. They do not inspect files, canonicalize paths,
//! persist grants, or execute effects; callers must enforce them at the effect boundary.

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkspaceId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(String);

/// Opaque, exact-match resource identity. This is not a canonical filesystem path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
}

macro_rules! identifier {
    ($type:ty) => {
        impl $type {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentifierError::Empty);
                }
                Ok(Self(value))
            }
        }
    };
}

identifier!(WorkspaceId);
identifier!(TaskId);
identifier!(ResourceId);

#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceTrust {
    workspace: WorkspaceId,
    trusted: bool,
}

impl WorkspaceTrust {
    /// Workspaces start untrusted. Repo-provided settings cannot construct trusted state.
    pub fn new(workspace: WorkspaceId) -> Self {
        Self {
            workspace,
            trusted: false,
        }
    }

    pub fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// Records an explicit operator decision; this does not approve any individual action.
    pub fn trust_by_operator(&mut self) {
        self.trusted = true;
    }

    pub fn revoke_by_operator(&mut self) {
        self.trusted = false;
    }

    /// Project configs, MCPs, and hooks remain inactive until the workspace is trusted.
    pub fn may_activate_project_extensions(&self) -> bool {
        self.trusted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    ReadResource,
    ModifyResource,
    RunCommand,
    ActivateProjectConfig,
    ActivateMcp,
    ActivateHook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationScope {
    Once,
    CurrentTask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationSource {
    OperatorApproval,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthorizationGrant {
    action: Action,
    resource: ResourceId,
    task: TaskId,
    scope: AuthorizationScope,
    source: AuthorizationSource,
    expires_at: u64,
    consumed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantError {
    ExpiryNotInFuture,
    Expired,
    WrongAction,
    WrongResource,
    WrongTask,
    AlreadyConsumed,
}

impl AuthorizationGrant {
    /// Creates an ephemeral grant from a direct operator approval.
    /// `expires_at` and `now` use the same caller-defined monotonic time unit.
    pub fn approved_by_operator(
        action: Action,
        resource: ResourceId,
        task: TaskId,
        scope: AuthorizationScope,
        expires_at: u64,
        now: u64,
    ) -> Result<Self, GrantError> {
        if expires_at <= now {
            return Err(GrantError::ExpiryNotInFuture);
        }
        Ok(Self {
            action,
            resource,
            task,
            scope,
            source: AuthorizationSource::OperatorApproval,
            expires_at,
            consumed: false,
        })
    }

    pub fn action(&self) -> Action {
        self.action
    }

    pub fn resource(&self) -> &ResourceId {
        &self.resource
    }

    pub fn task(&self) -> &TaskId {
        &self.task
    }

    pub fn scope(&self) -> AuthorizationScope {
        self.scope
    }

    pub fn source(&self) -> AuthorizationSource {
        self.source
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    /// Validates an exact action/resource/task match and consumes `Once` after success.
    /// `CurrentTask` grants remain valid only for the bound task and until expiry. A mismatch
    /// never consumes the grant. Grants are intentionally not cloneable, so a `Once` grant
    /// cannot be duplicated through this API.
    pub fn authorize(
        &mut self,
        action: Action,
        resource: &ResourceId,
        task: &TaskId,
        now: u64,
    ) -> Result<(), GrantError> {
        if now >= self.expires_at {
            return Err(GrantError::Expired);
        }
        if action != self.action {
            return Err(GrantError::WrongAction);
        }
        if resource != &self.resource {
            return Err(GrantError::WrongResource);
        }
        if task != &self.task {
            return Err(GrantError::WrongTask);
        }
        if self.scope == AuthorizationScope::Once && self.consumed {
            return Err(GrantError::AlreadyConsumed);
        }
        if self.scope == AuthorizationScope::Once {
            self.consumed = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, AuthorizationGrant, AuthorizationScope, AuthorizationSource, GrantError,
        IdentifierError, ResourceId, TaskId, WorkspaceId, WorkspaceTrust,
    };

    fn ids() -> (ResourceId, TaskId) {
        (
            ResourceId::new("workspace:alpha/file:src/main.rs").unwrap(),
            TaskId::new("task-17").unwrap(),
        )
    }

    #[test]
    fn should_require_operator_trust_before_project_extensions_can_be_activated() {
        let workspace = WorkspaceId::new("workspace:alpha").unwrap();
        let mut trust = WorkspaceTrust::new(workspace.clone());
        assert_eq!(trust.workspace(), &workspace);
        assert!(!trust.is_trusted());
        assert!(!trust.may_activate_project_extensions());

        trust.trust_by_operator();
        assert!(trust.is_trusted());
        assert!(trust.may_activate_project_extensions());

        trust.revoke_by_operator();
        assert!(!trust.may_activate_project_extensions());
    }

    #[test]
    fn should_bind_one_time_grants_to_exact_action_resource_task_and_expiry() {
        let (resource, task) = ids();
        let mut grant = AuthorizationGrant::approved_by_operator(
            Action::ModifyResource,
            resource.clone(),
            task.clone(),
            AuthorizationScope::Once,
            100,
            10,
        )
        .unwrap();
        assert_eq!(grant.source(), AuthorizationSource::OperatorApproval);
        assert_eq!(grant.expires_at(), 100);

        assert_eq!(
            grant.authorize(Action::ReadResource, &resource, &task, 11),
            Err(GrantError::WrongAction)
        );
        let other_resource = ResourceId::new("workspace:alpha/file:src/lib.rs").unwrap();
        assert_eq!(
            grant.authorize(Action::ModifyResource, &other_resource, &task, 11),
            Err(GrantError::WrongResource)
        );
        let other_task = TaskId::new("task-18").unwrap();
        assert_eq!(
            grant.authorize(Action::ModifyResource, &resource, &other_task, 11),
            Err(GrantError::WrongTask)
        );

        grant
            .authorize(Action::ModifyResource, &resource, &task, 99)
            .unwrap();
        assert_eq!(
            grant.authorize(Action::ModifyResource, &resource, &task, 99),
            Err(GrantError::AlreadyConsumed)
        );
        assert_eq!(
            grant.authorize(Action::ModifyResource, &resource, &task, 100),
            Err(GrantError::Expired)
        );
    }

    #[test]
    fn should_allow_current_task_grants_only_for_the_bound_task_until_expiry() {
        let (resource, task) = ids();
        let mut grant = AuthorizationGrant::approved_by_operator(
            Action::RunCommand,
            resource.clone(),
            task.clone(),
            AuthorizationScope::CurrentTask,
            50,
            1,
        )
        .unwrap();
        grant
            .authorize(Action::RunCommand, &resource, &task, 2)
            .unwrap();
        grant
            .authorize(Action::RunCommand, &resource, &task, 49)
            .unwrap();
        assert_eq!(
            grant.authorize(Action::RunCommand, &resource, &task, 50),
            Err(GrantError::Expired)
        );
        assert_eq!(
            AuthorizationGrant::approved_by_operator(
                Action::RunCommand,
                resource.clone(),
                task.clone(),
                AuthorizationScope::CurrentTask,
                50,
                50,
            ),
            Err(GrantError::ExpiryNotInFuture)
        );
        assert_eq!(grant.scope(), AuthorizationScope::CurrentTask);
    }

    #[test]
    fn should_reject_blank_identifiers() {
        assert_eq!(WorkspaceId::new(" \t"), Err(IdentifierError::Empty));
        assert_eq!(TaskId::new(""), Err(IdentifierError::Empty));
        assert_eq!(ResourceId::new("  "), Err(IdentifierError::Empty));
    }
}
