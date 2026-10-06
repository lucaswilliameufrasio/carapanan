//! Structured, content-free trace records for local observability adapters.
//!
//! Records deliberately contain only closed enums and numeric counters. They cannot carry
//! prompts, paths, command output, provider data, or secret values. This module does not
//! configure a subscriber, write files, or send telemetry.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    SessionStarted,
    MessageQueued,
    TurnStarted,
    ApprovalRequested,
    ApprovalResolved,
    InterventionStaged,
    InterventionApplied,
    ValidationCompleted,
    TurnCompleted,
    WorkspaceTrustChanged,
    AuthorizationChecked,
}

impl TraceEvent {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SessionStarted => "session_started",
            Self::MessageQueued => "message_queued",
            Self::TurnStarted => "turn_started",
            Self::ApprovalRequested => "approval_requested",
            Self::ApprovalResolved => "approval_resolved",
            Self::InterventionStaged => "intervention_staged",
            Self::InterventionApplied => "intervention_applied",
            Self::ValidationCompleted => "validation_completed",
            Self::TurnCompleted => "turn_completed",
            Self::WorkspaceTrustChanged => "workspace_trust_changed",
            Self::AuthorizationChecked => "authorization_checked",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceLevel {
    Info,
    Warn,
    Error,
}

impl TraceLevel {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceResult {
    Succeeded,
    Pending,
    Denied,
    Failed,
}

impl TraceResult {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Pending => "pending",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    correlation: u64,
    event: TraceEvent,
    level: TraceLevel,
    result: TraceResult,
    elapsed_millis: Option<u64>,
}

impl TraceRecord {
    pub const fn new(
        correlation: u64,
        event: TraceEvent,
        level: TraceLevel,
        result: TraceResult,
        elapsed_millis: Option<u64>,
    ) -> Self {
        Self {
            correlation,
            event,
            level,
            result,
            elapsed_millis,
        }
    }

    pub const fn correlation(self) -> u64 {
        self.correlation
    }

    pub const fn event(self) -> TraceEvent {
        self.event
    }

    pub const fn level(self) -> TraceLevel {
        self.level
    }

    pub const fn result(self) -> TraceResult {
        self.result
    }

    pub const fn elapsed_millis(self) -> Option<u64> {
        self.elapsed_millis
    }
}

/// Implementations may export only fields represented by `TraceRecord`.
/// Local file rotation and any remote telemetry policy belong to the adapter.
pub trait TraceSink {
    fn record(&mut self, event: TraceRecord);
}

#[cfg(test)]
mod tests {
    use super::{TraceEvent, TraceLevel, TraceRecord, TraceResult, TraceSink};

    #[derive(Default)]
    struct CollectingSink(Vec<TraceRecord>);

    impl TraceSink for CollectingSink {
        fn record(&mut self, event: TraceRecord) {
            self.0.push(event);
        }
    }

    #[test]
    fn should_emit_stable_structured_names_without_dynamic_content_fields() {
        let record = TraceRecord::new(
            17,
            TraceEvent::AuthorizationChecked,
            TraceLevel::Warn,
            TraceResult::Denied,
            Some(4),
        );
        let mut sink = CollectingSink::default();
        sink.record(record);

        let stored = sink.0[0];
        assert_eq!(stored.correlation(), 17);
        assert_eq!(stored.event().name(), "authorization_checked");
        assert_eq!(stored.level().name(), "warn");
        assert_eq!(stored.result().name(), "denied");
        assert_eq!(stored.elapsed_millis(), Some(4));
    }

    #[test]
    fn should_represent_absent_timing_without_free_form_values() {
        let record = TraceRecord::new(
            0,
            TraceEvent::SessionStarted,
            TraceLevel::Info,
            TraceResult::Succeeded,
            None,
        );
        assert_eq!(record.event().name(), "session_started");
        assert_eq!(record.elapsed_millis(), None);
    }
}
