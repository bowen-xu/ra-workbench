use super::super::events::{
    decode_messages, legacy_context_snapshot_id, legacy_request_id, legacy_turn_id,
};
use super::*;
use research_agent_core::protocol::TurnOutcome;

#[test]
fn legacy_messages_fallback_and_event_shape_are_stable() {
    let turn = LegacyTurn {
        id: "not-a-uuid".into(),
        ordinal: 0,
        user_message: "问".into(),
        assistant_message: "答".into(),
        messages_json: "[]".into(),
        provider_id: "p".into(),
        model_id: "m".into(),
        reasoning_effort: None,
        status: "succeeded".into(),
        input_tokens: None,
        output_tokens: None,
        input_hash: None,
        output_hash: None,
        context_window_tokens: None,
        context_window_source: None,
        error_code: None,
        error_message: None,
        raw_request_json: None,
        raw_response_json: None,
        created_at: "now".into(),
        finished_at: None,
    };
    assert_eq!(decode_messages(&turn).len(), 2);
    assert!(matches!(
        turn_payloads("legacy", &turn).last(),
        Some(EventPayload::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        })
    ));
    assert_eq!(
        serde_json::to_value(turn_payloads("legacy", &turn)).unwrap(),
        serde_json::to_value(turn_payloads("legacy", &turn)).unwrap()
    );
    let context_id = legacy_context_snapshot_id("legacy", &turn.id);
    assert_eq!(
        context_id,
        match turn_payloads("legacy", &turn).first() {
            Some(EventPayload::TurnStarted {
                context_snapshot_id,
                ..
            }) => *context_snapshot_id,
            _ => panic!("missing turn start"),
        }
    );
    assert_eq!(context_id, legacy_context_snapshot_id("legacy", &turn.id));
    assert_eq!(
        legacy_turn_id("session", "not-a-uuid"),
        legacy_turn_id("session", "not-a-uuid")
    );
    assert_ne!(
        legacy_request_id("session", "one"),
        legacy_request_id("session", "two")
    );
}

#[test]
fn legacy_structured_messages_do_not_drop_denormalized_assistant_text() {
    let turn = LegacyTurn {
        id: "turn".into(),
        ordinal: 1,
        user_message: "问题".into(),
        assistant_message: "回答".into(),
        messages_json: r#"[{"role":"user","content":"问题"}]"#.into(),
        provider_id: "provider".into(),
        model_id: "model".into(),
        reasoning_effort: None,
        status: "succeeded".into(),
        input_tokens: None,
        output_tokens: None,
        input_hash: None,
        output_hash: None,
        context_window_tokens: None,
        context_window_source: None,
        error_code: None,
        error_message: None,
        raw_request_json: None,
        raw_response_json: None,
        created_at: "now".into(),
        finished_at: None,
    };
    assert_eq!(
        decode_messages(&turn),
        vec![
            ("user".to_owned(), "问题".to_owned()),
            ("assistant".to_owned(), "回答".to_owned()),
        ]
    );
}
