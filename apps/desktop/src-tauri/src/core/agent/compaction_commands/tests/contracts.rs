use super::*;

#[test]
fn ipc_uses_stable_host_fields_and_rejects_renderer_policy() {
    let mut value = serde_json::json!({
        "conversationId": Uuid::new_v4(), "requestId": Uuid::new_v4(),
        "contextSnapshotId": Uuid::new_v4(), "expectedRevision": 0,
        "model": {"providerId":"deepseek","modelId":"deepseek-v4-pro","revision":null},
        "host": {"kind":"readingNote","readingNoteId":Uuid::new_v4()}
    });
    assert!(serde_json::from_value::<StartRequest>(value.clone()).is_ok());
    value["systemInstructions"] = "renderer policy".into();
    assert!(serde_json::from_value::<StartRequest>(value.clone()).is_err());
    value.as_object_mut().unwrap().remove("systemInstructions");
    value["host"]["extra"] = true.into();
    assert!(serde_json::from_value::<StartRequest>(value).is_err());
    let error = serde_json::to_value(Error::HistoryChanged {
        expected: 0,
        actual: 9,
    })
    .unwrap();
    assert_eq!(
        error,
        serde_json::json!({"code":"historyChanged","reason":{"expected":0,"actual":9}})
    );
}

#[tokio::test]
async fn transport_rechecks_frozen_model_and_revoked_host_before_provider_call() {
    let f = fixture("/compact").await;
    let transport = transport(&f, false);
    let bound = BoundTransport {
        store: f.store.clone(),
        request: f.request.clone(),
        record: f.record.clone(),
        inner: transport.clone(),
    };
    let mut request = ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: f.record.input.model.clone(),
        messages: vec![ModelMessage::System("Synthetic rules".into())],
        tools: vec![],
        max_output_tokens: 10,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    request.model.model_id = "other-model".into();
    assert!(matches!(
        bound.start(request.clone()).await,
        Err(ModelError::InvalidRequest)
    ));
    request.model = f.record.input.model.clone();
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(matches!(
        bound.start(request).await,
        Err(ModelError::InvalidRequest)
    ));
    assert!(transport.requests.lock().unwrap().is_empty());
}
