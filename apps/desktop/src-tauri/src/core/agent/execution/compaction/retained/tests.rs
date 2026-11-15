use super::*;

fn user(text: &str) -> ModelMessage {
    ModelMessage::User {
        content: vec![ContentPart::Text(text.into())],
        source_item_id: None,
    }
}
fn image() -> ContentPart {
    ContentPart::Image {
        media_type: "image/png".into(),
        bytes: Arc::from(&b"synthetic frozen bytes"[..]),
    }
}
fn legacy() -> (ConversationProjection, ContextCheckpoint, Vec<ModelMessage>) {
    let id = Uuid::new_v4();
    let source = vec![
        user("first"),
        ModelMessage::User {
            content: vec![ContentPart::Text("old frozen rule".into()), image()],
            source_item_id: None,
        },
        user("newest"),
    ];
    let checkpoint = ContextCheckpoint {
        schema_version: 2,
        window_id: Uuid::new_v4(),
        conversation_id: id,
        turn_id: Uuid::new_v4(),
        source_watermark: 0,
        previous_window_id: None,
        binding: research_agent_core::context::compaction_strategy::CompactionBinding {
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            strategy_id: "fixture.v2".into(),
            state_schema: 1,
            kind: CompactionKind::RemoteOpaque,
        },
        credential_scope_id: None,
        phase: CompactionPhase::MidTurn,
        output: CompactionOutput::RemoteOpaque(
            serde_json::json!({"type":"compaction","encrypted_content":"synthetic"}),
        ),
        retained: store_messages(&[source[0].clone(), source[2].clone()]).unwrap(),
        canonical_context: Some(CanonicalContext {
            before_retained: 1,
            messages: store_messages(&[source[1].clone()]).unwrap(),
        }),
    };
    (
        ConversationProjection::new(ConversationId::from_uuid(id)),
        checkpoint,
        source,
    )
}

#[test]
fn legacy_canonical_images_keep_explicit_slot_without_old_rules_or_text_association() {
    let (history, mut checkpoint, source) = legacy();
    let retained = retained_users(
        &history,
        Some(&checkpoint),
        10_000,
        Some(&source),
        None,
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(retained.len(), 3);
    assert!(
        matches!(&retained[0], ModelMessage::User { content: parts, .. } if matches!(&parts[..], [ContentPart::Text(text)] if text == "first"))
    );
    assert!(
        matches!(&retained[1], ModelMessage::User { content: parts, .. } if matches!(&parts[..], [ContentPart::Image { .. }]))
    );
    assert!(
        matches!(&retained[2], ModelMessage::User { content: parts, .. } if matches!(&parts[..], [ContentPart::Text(text)] if text == "newest"))
    );
    checkpoint.retained = store_messages(&retained).unwrap();
    checkpoint.canonical_context = None;
    let again = retained_users(
        &history,
        Some(&checkpoint),
        10_000,
        Some(&retained),
        None,
        &HashMap::new(),
    )
    .unwrap();
    assert!(store_messages(&again).unwrap() == checkpoint.retained);
}

#[test]
fn referenced_legacy_image_missing_from_authorized_source_fails() {
    let (history, checkpoint, _) = legacy();
    assert!(
        retained_users(
            &history,
            Some(&checkpoint),
            10_000,
            Some(&[user("newest")]),
            None,
            &HashMap::new()
        )
        .is_err()
    );
}

#[test]
fn newest_retained_attachment_without_text_policy_cannot_be_silently_dropped() {
    let (history, mut checkpoint, _) = legacy();
    checkpoint.canonical_context = None;
    let source = vec![ModelMessage::User {
        content: vec![image()],
        source_item_id: None,
    }];
    checkpoint.retained = store_messages(&source).unwrap();
    assert!(matches!(
        retained_users(
            &history,
            Some(&checkpoint),
            1,
            Some(&source),
            None,
            &HashMap::new()
        ),
        Err(RunError::Budget)
    ));
}
