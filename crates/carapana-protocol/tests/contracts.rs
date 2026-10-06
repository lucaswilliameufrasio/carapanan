use carapana_protocol::*;

#[test]
fn should_round_trip_captured_messages_without_permissions_or_secret_fields() {
    let selection = Selection {
        profile: "ask".into(),
        work: WorkMode::Execute,
        autonomy: Autonomy::Ask,
        provider: "mock".into(),
        model: "gpt-mock".into(),
        variant: "default".into(),
    };
    let queued = Envelope::new(QueuedMessage {
        id: "message-1".into(),
        text: "hello".into(),
        origin: "tui".into(),
        selection,
    });
    let json = serde_json::to_string(&queued).unwrap();
    assert_eq!(
        serde_json::from_str::<Envelope<QueuedMessage>>(&json).unwrap(),
        queued
    );
    let mut injected: serde_json::Value = serde_json::from_str(&json).unwrap();
    injected["payload"]["selection"]["token"] = "not-a-real-token".into();
    assert!(serde_json::from_value::<Envelope<QueuedMessage>>(injected).is_err());
}

#[test]
fn should_reject_missing_unsupported_versions_unknown_fields_and_unknown_variants() {
    for json in [
        r#"{"protocol":2,"payload":"value"}"#,
        r#"{"payload":"value"}"#,
        r#"{"protocol":1,"payload":"value","extra":true}"#,
    ] {
        assert!(serde_json::from_str::<Envelope<String>>(json).is_err());
    }
    assert!(serde_json::from_str::<Outcome>(r#""unknown""#).is_err());
    assert!(serde_json::from_str::<Autonomy>(r#""unrestricted""#).is_err());
}

#[test]
fn should_preserve_headless_exit_codes_and_typed_event_error_round_trips() {
    for (outcome, code) in [
        (Outcome::Success, 0),
        (Outcome::Failure, 2),
        (Outcome::ValidationIncomplete, 3),
        (Outcome::ApprovalPending, 4),
    ] {
        assert_eq!(outcome.exit_code(), code);
        let event = Envelope::new(SessionEvent::Completed {
            message_id: "m1".into(),
            outcome,
        });
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            serde_json::from_str::<Envelope<SessionEvent>>(&json).unwrap(),
            event
        );
    }
    let error = ContractError {
        code: ErrorCode::AlreadyResolved,
        message_id: None,
    };
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        r#"{"code":"already_resolved","message_id":null}"#
    );
}
