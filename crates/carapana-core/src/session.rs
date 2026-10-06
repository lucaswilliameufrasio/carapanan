//! In-memory deterministic session lifecycle for contracts and mock frontends.
//!
//! This coordinator emits protocol events but does not call a provider, execute work,
//! persist state, or make authorization decisions.

use crate::queue::WorkQueue;
use carapana_protocol::{Outcome, QueuedMessage, SessionEvent};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    EmptyMessageId,
    DuplicateMessageId,
    AlreadyProcessing,
    NoActiveMessage,
    AlreadyPaused,
    NotPaused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveMessage {
    message: QueuedMessage,
    status: SessionStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEngine {
    queue: WorkQueue<QueuedMessage>,
    active: Option<ActiveMessage>,
    seen_message_ids: HashSet<String>,
}

impl Default for SessionEngine {
    fn default() -> Self {
        Self {
            queue: WorkQueue::new(),
            active: None,
            seen_message_ids: HashSet::new(),
        }
    }
}

impl SessionEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, message: QueuedMessage) -> Result<SessionEvent, SessionError> {
        if message.id.trim().is_empty() {
            return Err(SessionError::EmptyMessageId);
        }
        if !self.seen_message_ids.insert(message.id.clone()) {
            return Err(SessionError::DuplicateMessageId);
        }
        let event = SessionEvent::Queued {
            message_id: message.id.clone(),
        };
        self.queue.push(message);
        Ok(event)
    }

    /// Starts the oldest queued message. A caller must explicitly request each start.
    pub fn start_next(&mut self) -> Result<Option<SessionEvent>, SessionError> {
        if self.active.is_some() {
            return Err(SessionError::AlreadyProcessing);
        }
        let Some(message) = self.queue.take_next() else {
            return Ok(None);
        };
        let event = SessionEvent::Started {
            message_id: message.id.clone(),
        };
        self.active = Some(ActiveMessage {
            message,
            status: SessionStatus::Running,
        });
        Ok(Some(event))
    }

    pub fn pause(&mut self, reason: Outcome) -> Result<SessionEvent, SessionError> {
        let active = self.active.as_mut().ok_or(SessionError::NoActiveMessage)?;
        if active.status == SessionStatus::Paused {
            return Err(SessionError::AlreadyPaused);
        }
        active.status = SessionStatus::Paused;
        Ok(SessionEvent::Paused { reason })
    }

    pub fn resume(&mut self) -> Result<SessionEvent, SessionError> {
        let active = self.active.as_mut().ok_or(SessionError::NoActiveMessage)?;
        if active.status != SessionStatus::Paused {
            return Err(SessionError::NotPaused);
        }
        active.status = SessionStatus::Running;
        Ok(SessionEvent::Resumed {
            message_id: active.message.id.clone(),
        })
    }

    pub fn complete(&mut self, outcome: Outcome) -> Result<SessionEvent, SessionError> {
        let active = self.active.as_ref().ok_or(SessionError::NoActiveMessage)?;
        if active.status == SessionStatus::Paused {
            return Err(SessionError::AlreadyPaused);
        }
        let message_id = active.message.id.clone();
        self.active = None;
        Ok(SessionEvent::Completed {
            message_id,
            outcome,
        })
    }

    pub fn status(&self) -> Option<SessionStatus> {
        self.active.as_ref().map(|active| active.status)
    }

    pub fn active_message(&self) -> Option<&QueuedMessage> {
        self.active.as_ref().map(|active| &active.message)
    }

    pub fn queued_messages(&self) -> impl Iterator<Item = &QueuedMessage> {
        self.queue.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionEngine, SessionError, SessionStatus};
    use carapana_protocol::{Autonomy, Outcome, QueuedMessage, Selection, SessionEvent, WorkMode};

    fn message(id: &str) -> QueuedMessage {
        QueuedMessage {
            id: id.into(),
            text: format!("message {id}"),
            origin: "test".into(),
            selection: Selection {
                profile: "ask".into(),
                work: WorkMode::Execute,
                autonomy: Autonomy::Ask,
                provider: "mock".into(),
                model: "mock-model".into(),
                variant: "default".into(),
            },
        }
    }

    #[test]
    fn should_emit_queue_start_and_completion_events_in_fifo_order() {
        let mut engine = SessionEngine::new();
        assert_eq!(
            engine.enqueue(message("m1")),
            Ok(SessionEvent::Queued {
                message_id: "m1".into()
            })
        );
        engine.enqueue(message("m2")).unwrap();

        assert_eq!(
            engine.start_next(),
            Ok(Some(SessionEvent::Started {
                message_id: "m1".into()
            }))
        );
        assert_eq!(engine.status(), Some(SessionStatus::Running));
        assert_eq!(
            engine.active_message().map(|message| message.id.as_str()),
            Some("m1")
        );
        assert_eq!(engine.start_next(), Err(SessionError::AlreadyProcessing));
        assert_eq!(
            engine.complete(Outcome::Success),
            Ok(SessionEvent::Completed {
                message_id: "m1".into(),
                outcome: Outcome::Success,
            })
        );

        assert_eq!(
            engine.start_next(),
            Ok(Some(SessionEvent::Started {
                message_id: "m2".into()
            }))
        );
        assert_eq!(engine.queued_messages().count(), 0);
    }

    #[test]
    fn should_pause_and_resume_only_the_active_message() {
        let mut engine = SessionEngine::new();
        assert_eq!(
            engine.pause(Outcome::Failure),
            Err(SessionError::NoActiveMessage)
        );
        engine.enqueue(message("m1")).unwrap();
        engine.start_next().unwrap();

        assert_eq!(
            engine.pause(Outcome::ApprovalPending),
            Ok(SessionEvent::Paused {
                reason: Outcome::ApprovalPending,
            })
        );
        assert_eq!(engine.status(), Some(SessionStatus::Paused));
        assert_eq!(
            engine.complete(Outcome::Success),
            Err(SessionError::AlreadyPaused)
        );
        assert_eq!(
            engine.resume(),
            Ok(SessionEvent::Resumed {
                message_id: "m1".into()
            })
        );
        assert_eq!(engine.status(), Some(SessionStatus::Running));
        assert_eq!(engine.resume(), Err(SessionError::NotPaused));
    }

    #[test]
    fn should_reject_empty_and_reused_message_ids() {
        let mut engine = SessionEngine::new();
        assert_eq!(
            engine.enqueue(message("  ")),
            Err(SessionError::EmptyMessageId)
        );
        engine.enqueue(message("m1")).unwrap();
        assert_eq!(
            engine.enqueue(message("m1")),
            Err(SessionError::DuplicateMessageId)
        );
        engine.start_next().unwrap();
        engine.complete(Outcome::Success).unwrap();
        assert_eq!(
            engine.enqueue(message("m1")),
            Err(SessionError::DuplicateMessageId)
        );
    }

    #[test]
    fn should_not_start_when_the_queue_is_empty_or_complete_without_active_work() {
        let mut engine = SessionEngine::new();
        assert_eq!(engine.start_next(), Ok(None));
        assert_eq!(
            engine.complete(Outcome::Success),
            Err(SessionError::NoActiveMessage)
        );
    }
}
