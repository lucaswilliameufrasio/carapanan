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
    assert_eq!(queued.protocol(), PROTOCOL_VERSION);
    assert_eq!(
        serde_json::from_str::<Envelope<QueuedMessage>>(&json).unwrap(),
        queued
    );
    let mut injected: serde_json::Value = serde_json::from_str(&json).unwrap();
    injected["payload"]["selection"]["token"] = "not-a-real-token".into();
    assert!(serde_json::from_value::<Envelope<QueuedMessage>>(injected).is_err());
}

#[test]
fn should_round_trip_profile_defaults_without_permission_fields() {
    let profile = Profile {
        id: "ask-default".into(),
        name: "Perguntar".into(),
        work: WorkMode::Execute,
        autonomy: Autonomy::Ask,
        provider: "mock-provider".into(),
        model: "mock-model".into(),
        variant: "default".into(),
    };
    let json = serde_json::to_string(&Envelope::new(profile.clone())).unwrap();
    assert_eq!(
        serde_json::from_str::<Envelope<Profile>>(&json).unwrap(),
        Envelope::new(profile)
    );
    assert!(!json.contains("permission"));
    assert!(!json.contains("secret"));
    let mut injected: serde_json::Value = serde_json::from_str(&json).unwrap();
    injected["payload"]["permissions"] = serde_json::json!(["all"]);
    assert!(serde_json::from_value::<Envelope<Profile>>(injected).is_err());
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
    assert_eq!(
        serde_json::to_string(&ContractError {
            code: ErrorCode::DuplicateMessageId,
            message_id: Some("message-7".into()),
        })
        .unwrap(),
        r#"{"code":"duplicate_message_id","message_id":"message-7"}"#
    );
}

#[test]
fn should_round_trip_the_resumed_session_event() {
    let event = Envelope::new(SessionEvent::Resumed {
        message_id: "m1".into(),
    });
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(
        json,
        r#"{"protocol":1,"payload":{"type":"resumed","message_id":"m1"}}"#
    );
    assert_eq!(
        serde_json::from_str::<Envelope<SessionEvent>>(&json).unwrap(),
        event
    );
}

#[test]
fn should_version_read_only_daemon_session_listing_and_reject_unknown_fields() {
    let request = Envelope::new(DaemonRequest::ListSessions {});
    let request_json = serde_json::to_string(&request).unwrap();
    assert_eq!(
        request_json,
        r#"{"protocol":1,"payload":{"type":"list_sessions"}}"#
    );
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonRequest>>(&request_json).unwrap(),
        request
    );

    let response = Envelope::new(DaemonResponse::Sessions {
        sessions: vec![SessionSummary {
            session_id: "session-1".into(),
            status: SessionStatus::Paused,
            queued_count: 2,
            has_active_message: true,
            recovery_needs_revalidation: true,
            active_work_uncertain: true,
            attached_clients: 0,
            updated_at_ms: 42,
        }],
    });
    let response_json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonResponse>>(&response_json).unwrap(),
        response
    );
    assert!(!response_json.contains("text"));
    assert!(!response_json.contains("selection"));
    assert!(!response_json.contains("secret"));

    for invalid in [
        r#"{"protocol":2,"payload":{"type":"list_sessions"}}"#,
        r#"{"protocol":1,"payload":{"type":"list_sessions","execute":true}}"#,
        r#"{"protocol":1,"payload":{"type":"run_command"}}"#,
    ] {
        assert!(
            serde_json::from_str::<Envelope<DaemonRequest>>(invalid).is_err(),
            "accepted invalid request: {invalid}"
        );
    }
    assert!(serde_json::from_str::<SessionSummary>(
        r#"{"session_id":"s","status":"paused","queued_count":0,"has_active_message":false,"recovery_needs_revalidation":false,"active_work_uncertain":false,"attached_clients":0,"updated_at_ms":0,"prompt":"not allowed"}"#
    )
    .is_err());
}

#[test]
fn should_model_connection_scoped_attach_with_a_full_reconnect_snapshot() {
    let attach = Envelope::new(DaemonRequest::Attach {
        session_id: "session-1".into(),
    });
    let attach_json = serde_json::to_string(&attach).unwrap();
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonRequest>>(&attach_json).unwrap(),
        attach
    );
    assert!(
        serde_json::from_str::<Envelope<DaemonRequest>>(
            r#"{"protocol":1,"payload":{"type":"detach","session_id":"spoofed"}}"#
        )
        .is_err()
    );

    let response = Envelope::new(DaemonResponse::Attached {
        snapshot: Box::new(SessionSnapshot {
            session_id: "session-1".into(),
            status: SessionStatus::Paused,
            queued_messages: vec![QueuedMessage {
                id: "queued-1".into(),
                text: "preserve this queued message".into(),
                origin: "tui".into(),
                selection: Selection {
                    profile: "ask".into(),
                    work: WorkMode::Plan,
                    autonomy: Autonomy::Ask,
                    provider: "mock".into(),
                    model: "mock-model".into(),
                    variant: "default".into(),
                },
            }],
            active_message: None,
            recovery_needs_revalidation: false,
            active_work_uncertain: false,
            attached_clients: 1,
            created_at_ms: 10,
            updated_at_ms: 11,
            event_sequence: 2,
        }),
    });
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("preserve this queued message"));
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonResponse>>(&json).unwrap(),
        response
    );
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonRequest>>(
            r#"{"protocol":1,"payload":{"type":"detach"}}"#
        )
        .unwrap()
        .payload,
        DaemonRequest::Detach {}
    );
}

#[test]
fn should_round_trip_derived_recovery_attention_without_free_form_details() {
    let request = Envelope::new(DaemonRequest::ListAttention {});
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonRequest>>(&serde_json::to_string(&request).unwrap())
            .unwrap(),
        request
    );

    let response = Envelope::new(DaemonResponse::Attention {
        items: vec![AttentionItem {
            session_id: "session-1".into(),
            reason: AttentionReason::RecoveryReview,
            active_work_uncertain: true,
            updated_at_ms: 20,
            event_sequence: 4,
        }],
    });
    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains("secret"));
    assert_eq!(
        serde_json::from_str::<Envelope<DaemonResponse>>(&json).unwrap(),
        response
    );
}
