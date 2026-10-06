//! Extracted Delivery 0 script. Pure deterministic decisions, no tools or IO.
use carapana_protocol::{Autonomy, WorkMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoRequest {
    pub step: u8,
    pub work: WorkMode,
    pub autonomy: Autonomy,
    pub explicit_demo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoResponse {
    Acknowledge,
    Read,
    Plan,
    PlanComplete,
    ApprovalRequired,
    MockPolicyAllows,
    ApprovedActivity,
    Validation,
    Complete,
}

/// A mock-only boundary. This is not the real inference-provider API.
pub trait DemoProvider {
    fn next(&self, request: DemoRequest) -> DemoResponse;
}

pub struct DeterministicDemoProvider;

impl DemoProvider for DeterministicDemoProvider {
    fn next(&self, request: DemoRequest) -> DemoResponse {
        if !request.explicit_demo {
            return DemoResponse::Acknowledge;
        }
        match request.step {
            0 => DemoResponse::Read,
            1 => DemoResponse::Plan,
            // Plan can never acquire edit/test activity, even for later steps.
            _ if request.work == WorkMode::Plan => DemoResponse::PlanComplete,
            2 if request.autonomy == Autonomy::Ask => DemoResponse::ApprovalRequired,
            2 => DemoResponse::MockPolicyAllows,
            3 => DemoResponse::ApprovedActivity,
            4 => DemoResponse::Validation,
            _ => DemoResponse::Complete,
        }
    }
}
