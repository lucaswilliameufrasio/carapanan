use carapana_core::*;
use carapana_protocol::{Autonomy, WorkMode};

#[test]
fn should_preserve_the_existing_explicit_demo_sequence_for_each_preset() {
    let provider = DeterministicDemoProvider;
    for autonomy in [Autonomy::Ask, Autonomy::Auto, Autonomy::Yolo] {
        let responses: Vec<_> = (0..=5)
            .map(|step| {
                provider.next(DemoRequest {
                    step,
                    work: WorkMode::Execute,
                    autonomy,
                    explicit_demo: true,
                })
            })
            .collect();
        assert_eq!(
            responses,
            vec![
                DemoResponse::Read,
                DemoResponse::Plan,
                if autonomy == Autonomy::Ask {
                    DemoResponse::ApprovalRequired
                } else {
                    DemoResponse::MockPolicyAllows
                },
                DemoResponse::ApprovedActivity,
                DemoResponse::Validation,
                DemoResponse::Complete
            ]
        );
    }
}

#[test]
fn should_never_treat_ordinary_input_or_plan_mode_as_permission_to_execute() {
    for step in 0..=u8::MAX {
        for autonomy in [Autonomy::Ask, Autonomy::Auto, Autonomy::Yolo] {
            let request = DemoRequest {
                step,
                work: WorkMode::Plan,
                autonomy,
                explicit_demo: false,
            };
            assert_eq!(
                DeterministicDemoProvider.next(request),
                DemoResponse::Acknowledge
            );
            if step >= 2 {
                assert_eq!(
                    DeterministicDemoProvider.next(DemoRequest {
                        explicit_demo: true,
                        ..request
                    }),
                    DemoResponse::PlanComplete
                );
            }
        }
    }
}
