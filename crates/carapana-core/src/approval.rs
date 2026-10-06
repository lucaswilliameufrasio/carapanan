//! Explicit approval state transitions. Approval state is not authorization policy.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalState {
    #[default]
    NotRequired,
    Pending,
    Approved,
    Denied,
    Invalidated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    NotPending,
    Invalidated,
}

impl ApprovalState {
    /// Records a fresh request after the action has been re-evaluated.
    pub fn request(&mut self) {
        *self = Self::Pending;
    }

    /// Invalidates only an unresolved request. A resolved decision is unchanged.
    pub fn invalidate(&mut self) -> bool {
        if *self == Self::Pending {
            *self = Self::Invalidated;
            true
        } else {
            false
        }
    }

    /// Resolves a pending request; stale or duplicate resolutions fail closed.
    pub fn resolve(&mut self, allow: bool) -> Result<(), ApprovalError> {
        match self {
            Self::Pending => {
                *self = if allow { Self::Approved } else { Self::Denied };
                Ok(())
            }
            Self::Invalidated => Err(ApprovalError::Invalidated),
            Self::NotRequired | Self::Approved | Self::Denied => Err(ApprovalError::NotPending),
        }
    }

    /// A denied action still requires a fresh request when the session resumes.
    pub fn requires_approval(self) -> bool {
        matches!(self, Self::Pending | Self::Denied)
    }

    pub fn is_invalidated(self) -> bool {
        self == Self::Invalidated
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalError, ApprovalState};

    #[test]
    fn should_resolve_only_a_current_request() {
        let mut approval = ApprovalState::default();
        assert_eq!(approval.resolve(true), Err(ApprovalError::NotPending));

        approval.request();
        approval.resolve(false).unwrap();
        assert_eq!(approval, ApprovalState::Denied);
        assert!(approval.requires_approval());

        approval.request();
        approval.resolve(true).unwrap();
        assert_eq!(approval, ApprovalState::Approved);
        assert!(!approval.requires_approval());
        assert_eq!(approval.resolve(true), Err(ApprovalError::NotPending));
    }

    #[test]
    fn should_reject_a_stale_approval_after_intervention() {
        let mut approval = ApprovalState::default();
        approval.request();

        assert!(approval.invalidate());
        assert!(approval.is_invalidated());
        assert!(!approval.invalidate());
        assert_eq!(approval.resolve(true), Err(ApprovalError::Invalidated));
        assert!(approval.is_invalidated());
    }
}
