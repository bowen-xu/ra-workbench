use super::*;
use research_agent_core::context::{checkpoint::*, compaction_strategy::*};

fn reset(request: &ModelRequest) -> ContextCheckpoint {
    ContextCheckpoint {
        schema_version: 3,
        window_id: uuid::Uuid::new_v4(),
        conversation_id: uuid::Uuid::new_v4(),
        turn_id: uuid::Uuid::new_v4(),
        source_watermark: 0,
        previous_window_id: None,
        binding: CompactionBinding {
            model: request.model.clone(),
            strategy_id: "fixture.reset".into(),
            state_schema: 1,
            kind: CompactionKind::ResetWindow,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: store_messages(
                &request
                    .messages
                    .iter()
                    .filter(|item| matches!(item, ModelMessage::Developer(_)))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        }),
    }
}

#[tokio::test]
async fn declared_exact_model_encodes_developer_after_reset_on_actual_http() {
    let body = format!(
        "{}data: [DONE]\n\n",
        data(json!({"content":"ok"}), Some("stop"))
    );
    let (url, server) = server(response(&body), false).await;
    let mut request = request();
    request.model.revision = Some("reviewed-v1".into());
    request.messages = vec![
        ModelMessage::System("base".into()),
        ModelMessage::Developer("trusted reset guidance".into()),
        ModelMessage::User {
            content: vec![ContentPart::Text("new input".into())],
            source_item_id: None,
        },
    ];
    let saved = serde_json::to_vec(&reset(&request)).unwrap();
    let restored: ContextCheckpoint = serde_json::from_slice(&saved).unwrap();
    request.messages = vec![ModelMessage::System("base".into())];
    request
        .messages
        .extend(restored.restore_reset_window(&|_| None).unwrap());
    request.messages.push(ModelMessage::User {
        content: vec![ContentPart::Text("new input".into())],
        source_item_id: None,
    });
    let transport = transport(url)
        .with_developer_role(request.model.clone())
        .unwrap();
    transport
        .validate_compaction_window(&restored, &request)
        .await
        .unwrap();
    let mut stream = transport.start(request).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    drop(stream);
    let (_, sent) = server.await.unwrap();
    assert_eq!(
        sent["messages"][0],
        json!({"role":"system","content":"base"})
    );
    assert_eq!(
        sent["messages"][1],
        json!({"role":"developer","content":"trusted reset guidance"})
    );
    assert_eq!(sent["messages"][2]["role"], "user");
}

#[tokio::test]
async fn developer_declaration_cannot_cross_model_revision_or_provider_before_io() {
    let mut request = request();
    request.messages = vec![ModelMessage::Developer("trusted reset guidance".into())];
    request.model.revision = Some("reviewed-v1".into());
    let transport = transport("https://fixture.invalid/chat/completions".into())
        .with_developer_role(request.model.clone())
        .unwrap();
    for change in 0..3 {
        let mut other = request.clone();
        match change {
            0 => other.model.model_id = "another".into(),
            1 => other.model.revision = Some("reviewed-v2".into()),
            _ => other.model.provider_id = "another-provider".into(),
        }
        assert!(
            transport
                .validate_compaction_window(&reset(&other), &other)
                .await
                .is_err()
        );
        assert!(transport.start(other).await.is_err());
    }
    assert!(
        super::transport("https://fixture.invalid/chat/completions".into())
            .with_developer_role(ModelIdentity {
                provider_id: "wrong-provider".into(),
                ..request.model
            })
            .is_err()
    );
}
