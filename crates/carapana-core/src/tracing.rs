//! Structured, content-free trace records for local observability adapters.
//!
//! Records deliberately contain only closed enums and numeric counters. They cannot carry
//! prompts, paths, command output, provider data, or secret values. This module does not
//! configure a subscriber, write files, or send telemetry.

use carapana_protocol::{Outcome, SessionEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    SessionStarted,
    MessageQueued,
    TurnStarted,
    SessionResumed,
    SessionPaused,
    ApprovalRequested,
    ApprovalResolved,
    ApprovalInvalidated,
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
            Self::SessionResumed => "session_resumed",
            Self::SessionPaused => "session_paused",
            Self::ApprovalRequested => "approval_requested",
            Self::ApprovalResolved => "approval_resolved",
            Self::ApprovalInvalidated => "approval_invalidated",
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

impl From<Outcome> for TraceResult {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Success => Self::Succeeded,
            Outcome::Failure | Outcome::ValidationIncomplete => Self::Failed,
            Outcome::ApprovalPending => Self::Pending,
        }
    }
}

impl From<TraceResult> for TraceLevel {
    fn from(result: TraceResult) -> Self {
        match result {
            TraceResult::Succeeded | TraceResult::Pending => Self::Info,
            TraceResult::Denied => Self::Warn,
            TraceResult::Failed => Self::Error,
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

    /// Projects a protocol event into a content-free observability record.
    /// Message/approval identifiers are intentionally ignored; callers provide a
    /// numeric correlation value that is safe for their local trace boundary.
    pub fn from_session_event(correlation: u64, event: &SessionEvent) -> Self {
        let (event, result) = match event {
            SessionEvent::Queued { .. } => (TraceEvent::MessageQueued, TraceResult::Pending),
            SessionEvent::Started { .. } => (TraceEvent::TurnStarted, TraceResult::Succeeded),
            SessionEvent::Resumed { .. } => (TraceEvent::SessionResumed, TraceResult::Succeeded),
            SessionEvent::Paused { reason } => {
                (TraceEvent::SessionPaused, TraceResult::from(*reason))
            }
            SessionEvent::ApprovalInvalidated { .. } => {
                (TraceEvent::ApprovalInvalidated, TraceResult::Denied)
            }
            SessionEvent::InterventionApplied { .. } => {
                (TraceEvent::InterventionApplied, TraceResult::Succeeded)
            }
            SessionEvent::Completed { outcome, .. } => {
                (TraceEvent::TurnCompleted, TraceResult::from(*outcome))
            }
        };
        Self::new(correlation, event, TraceLevel::from(result), result, None)
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

    fn record_session_event(&mut self, correlation: u64, event: &SessionEvent) {
        self.record(TraceRecord::from_session_event(correlation, event));
    }
}

/// Emits only the closed, content-free fields represented by [`TraceRecord`]
/// through the application's local `tracing` subscriber.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTraceSink;

impl TraceSink for LocalTraceSink {
    fn record(&mut self, record: TraceRecord) {
        let event = record.event().name();
        let result = record.result().name();
        let correlation = record.correlation();
        let elapsed_millis = record.elapsed_millis().unwrap_or_default();
        let elapsed_millis_present = record.elapsed_millis().is_some();

        match record.level() {
            TraceLevel::Info => tracing::info!(
                target: "carapana::trace",
                correlation,
                event,
                result,
                elapsed_millis,
                elapsed_millis_present,
                "trace event"
            ),
            TraceLevel::Warn => tracing::warn!(
                target: "carapana::trace",
                correlation,
                event,
                result,
                elapsed_millis,
                elapsed_millis_present,
                "trace event"
            ),
            TraceLevel::Error => tracing::error!(
                target: "carapana::trace",
                correlation,
                event,
                result,
                elapsed_millis,
                elapsed_millis_present,
                "trace event"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceEvent, TraceLevel, TraceRecord, TraceResult, TraceSink};
    use carapana_protocol::{Outcome, SessionEvent};

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

    #[test]
    fn should_project_every_session_event_without_copying_identifiers_into_traces() {
        let events = [
            (
                SessionEvent::Queued {
                    message_id: "private-message-id".into(),
                },
                TraceEvent::MessageQueued,
                TraceResult::Pending,
            ),
            (
                SessionEvent::Started {
                    message_id: "private-message-id".into(),
                },
                TraceEvent::TurnStarted,
                TraceResult::Succeeded,
            ),
            (
                SessionEvent::Resumed {
                    message_id: "private-message-id".into(),
                },
                TraceEvent::SessionResumed,
                TraceResult::Succeeded,
            ),
            (
                SessionEvent::Paused {
                    reason: Outcome::ApprovalPending,
                },
                TraceEvent::SessionPaused,
                TraceResult::Pending,
            ),
            (
                SessionEvent::ApprovalInvalidated {
                    approval_id: "private-approval-id".into(),
                },
                TraceEvent::ApprovalInvalidated,
                TraceResult::Denied,
            ),
            (
                SessionEvent::InterventionApplied {
                    message_id: "private-message-id".into(),
                },
                TraceEvent::InterventionApplied,
                TraceResult::Succeeded,
            ),
            (
                SessionEvent::Completed {
                    message_id: "private-message-id".into(),
                    outcome: Outcome::ValidationIncomplete,
                },
                TraceEvent::TurnCompleted,
                TraceResult::Failed,
            ),
        ];

        let mut sink = CollectingSink::default();
        for (event, _, _) in &events {
            sink.record_session_event(23, event);
        }

        assert_eq!(sink.0.len(), events.len());
        for (record, (_, expected_event, expected_result)) in sink.0.iter().zip(events.iter()) {
            assert_eq!(record.correlation(), 23);
            assert_eq!(record.event(), *expected_event);
            assert_eq!(record.result(), *expected_result);
            assert_eq!(record.level(), TraceLevel::from(*expected_result));
            assert_eq!(record.elapsed_millis(), None);
            let debug_record = format!("{record:?}");
            assert!(!debug_record.contains("private-message-id"));
            assert!(!debug_record.contains("private-approval-id"));
        }
    }

    #[test]
    fn should_reduce_each_outcome_to_a_closed_trace_result_and_level() {
        for (outcome, result, level) in [
            (Outcome::Success, TraceResult::Succeeded, TraceLevel::Info),
            (Outcome::Failure, TraceResult::Failed, TraceLevel::Error),
            (
                Outcome::ValidationIncomplete,
                TraceResult::Failed,
                TraceLevel::Error,
            ),
            (
                Outcome::ApprovalPending,
                TraceResult::Pending,
                TraceLevel::Info,
            ),
        ] {
            let record = TraceRecord::from_session_event(
                0,
                &SessionEvent::Completed {
                    message_id: "message".into(),
                    outcome,
                },
            );
            assert_eq!(record.result(), result);
            assert_eq!(record.level(), level);
        }
    }
}
