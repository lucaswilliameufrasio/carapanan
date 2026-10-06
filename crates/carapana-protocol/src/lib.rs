//! Delivery 1 contracts only. No transport, authorization or task execution.
use serde::{Deserialize, Deserializer, Serialize, de::Error};

pub const PROTOCOL_VERSION: u16 = 1;

fn version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    let value = u16::deserialize(deserializer)?;
    if value != PROTOCOL_VERSION {
        return Err(D::Error::custom("unsupported_protocol_version"));
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope<T> {
    #[serde(deserialize_with = "version")]
    protocol: u16,
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(payload: T) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            payload,
        }
    }

    pub fn protocol(&self) -> u16 {
        self.protocol
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    Plan,
    Execute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    Ask,
    Auto,
    Yolo,
}

/// Stored profile defaults. A profile describes selections, never permissions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub work: WorkMode,
    pub autonomy: Autonomy,
    pub provider: String,
    pub model: String,
    pub variant: String,
}

/// Captured selection is data, never a permission grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub profile: String,
    pub work: WorkMode,
    pub autonomy: Autonomy,
    pub provider: String,
    pub model: String,
    pub variant: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    pub origin: String,
    pub selection: Selection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
    ValidationIncomplete,
    ApprovalPending,
}

impl Outcome {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 2,
            Self::ValidationIncomplete => 3,
            Self::ApprovalPending => 4,
        }
    }
}

/// Legacy Delivery 0 JSON shape, preserved while versioned contracts are added.
#[derive(Debug, Serialize)]
pub struct HeadlessResult {
    pub prototype: bool,
    pub executed: bool,
    pub scenario: String,
    pub status: String,
    pub summary: String,
    pub exit_code: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidSelection,
    InvalidMessageId,
    DuplicateMessageId,
    AlreadyProcessing,
    NoActiveMessage,
    AlreadyPaused,
    NotPaused,
    AlreadyResolved,
    ApprovalInvalidated,
    ProviderUnavailable,
    ValidationIncomplete,
}

/// Context contains identifiers, not prompts, file contents or credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractError {
    pub code: ErrorCode,
    pub message_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionEvent {
    Queued {
        message_id: String,
    },
    Started {
        message_id: String,
    },
    Resumed {
        message_id: String,
    },
    Paused {
        reason: Outcome,
    },
    ApprovalInvalidated {
        approval_id: String,
    },
    InterventionApplied {
        message_id: String,
    },
    Completed {
        message_id: String,
        outcome: Outcome,
    },
}
