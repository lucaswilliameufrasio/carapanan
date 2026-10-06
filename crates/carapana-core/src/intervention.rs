//! Pending intervention state; applying it is only possible at a safe step.

pub struct PendingIntervention<T> {
    pending: Option<T>,
}

impl<T> Default for PendingIntervention<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PendingIntervention<T> {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn get(&self) -> Option<&T> {
        self.pending.as_ref()
    }

    /// Stages the newest request and returns any displaced request for preservation.
    pub fn stage(&mut self, intervention: T) -> Option<T> {
        self.pending.replace(intervention)
    }

    /// Consumes a request only when the caller has reached a safe step.
    pub fn take_at_safe_step(&mut self) -> Option<T> {
        self.pending.take()
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::PendingIntervention;

    #[test]
    fn should_keep_replaced_requests_available_for_preservation() {
        let mut intervention = PendingIntervention::new();
        assert!(intervention.stage("first").is_none());
        assert_eq!(intervention.stage("second"), Some("first"));
        assert_eq!(intervention.get(), Some(&"second"));
        assert_eq!(intervention.take_at_safe_step(), Some("second"));
        assert!(!intervention.has_pending());
        assert_eq!(intervention.take_at_safe_step(), None);
    }
}
