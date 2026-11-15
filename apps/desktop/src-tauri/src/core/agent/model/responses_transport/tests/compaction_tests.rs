use super::*;
use research_agent_core::context::compaction_strategy::{
    CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase, CompactionRequest,
    CompactionStrategy, prepare_compaction,
};

pub(super) fn compact_request() -> CompactionRequest {
    CompactionRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        history_revision: 1,
        phase: CompactionPhase::Manual,
        binding: CompactionBinding {
            model: request().model,
            strategy_id: "fixture.responses.v2".into(),
            state_schema: 1,
            kind: CompactionKind::RemoteOpaque,
        },
        source: request(),
        max_result_bytes: 4096,
    }
}
pub(super) fn events(items: &[Value], completed: bool) -> String {
    let mut values = vec![
        json!({"type":"response.created","response":{"id":"compact1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}}),
    ];
    for item in items {
        values.push(json!({"type":"response.output_item.done","item":item}));
    }
    if completed {
        // This dedicated collector, like pinned CLI V2, obtains payloads from
        // done items, not from the response.completed output array.
        values.push(json!({"type":"response.completed","response":{
            "id":"compact1","model":"fixture-model","status":"completed","output":[],
            "usage":{"input_tokens":80,"output_tokens":4}}}));
    }
    values
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}
pub(super) fn opaque() -> Value {
    // The fixed protocol permits a missing item id and empty encrypted string.
    // Do not fabricate an id or claim semantic validity from payload content.
    json!({"type":"compaction","encrypted_content":"opaque fixture","vendor_field":{"version":1}})
}

#[tokio::test]
async fn reset_window_validation_needs_no_opaque_state_or_model_request() {
    use research_agent_core::context::checkpoint::{CanonicalContext, ContextCheckpoint};
    let sink = Arc::new(Receipts::default());
    let transport = transport("http://127.0.0.1:1/responses".into(), sink.clone());
    let mut binding = compact_request().binding;
    binding.kind = CompactionKind::ResetWindow;
    binding.strategy_id = "token-budget.reset-window".into();
    let window = ContextCheckpoint {
        schema_version: 2,
        window_id: Uuid::now_v7(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        source_watermark: 0,
        previous_window_id: None,
        binding,
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: vec![],
        }),
    };
    transport
        .validate_compaction_window(&window, &request())
        .await
        .unwrap();
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 0);
    assert!(sink.values.lock().unwrap().is_empty());
    let mut incompatible = request();
    incompatible.model.revision = Some("different".into());
    assert!(matches!(
        transport
            .validate_compaction_window(&window, &incompatible)
            .await,
        Err(ModelError::UnsupportedCapability)
    ));
}

#[tokio::test]
async fn reset_window_validates_native_history_scope_before_installation() {
    use crate::agent::model::responses_native::{NativeScope, capture};
    use research_agent_core::context::checkpoint::{CanonicalContext, ContextCheckpoint};
    let (lease, epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("window-account");
    let account = lease.scope_id();
    let sink = Arc::new(Receipts::default());
    let mut transport = transport("http://127.0.0.1:1/responses".into(), sink.clone());
    transport.kind = ResponsesEndpointKind::ChatGpt;
    transport.config.provider_id = "openai".into();
    transport.auth = Authentication::SyntheticLease(Arc::new(lease));
    let mut source = request();
    source.model.provider_id = "openai".into();
    source.model.model_id = "new-model".into();
    let old_model = ModelIdentity {
        model_id: "old-model".into(),
        ..source.model.clone()
    };
    let scope = NativeScope {
        model: old_model,
        account_scope: account,
    };
    let original = json!({"type":"function_call","id":"native-call","call_id":"call","name":"find_doc","namespace":"mcp__literature","arguments":"{}"});
    let item = capture(&original, &scope, None, None).unwrap();
    source
        .messages
        .push(ModelMessage::CanonicalOutput(Box::new(item.clone())));
    source.messages.push(ModelMessage::ToolResult {
        call_id: "call".into(),
        content: "synthetic".into(),
        is_error: false,
    });
    let mut binding = compact_request().binding;
    binding.model = source.model.clone();
    binding.kind = CompactionKind::ResetWindow;
    binding.strategy_id = "token-budget.reset-window".into();
    let window = ContextCheckpoint {
        schema_version: 2,
        window_id: Uuid::now_v7(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        source_watermark: 0,
        previous_window_id: None,
        binding,
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: vec![],
        }),
    };
    transport
        .validate_compaction_window(&window, &source)
        .await
        .unwrap();
    let mut foreign = source.clone();
    let ModelMessage::CanonicalOutput(item) = &mut foreign.messages[1] else {
        unreachable!()
    };
    item.native.as_mut().unwrap().account_scope = Some(Uuid::new_v4());
    assert!(matches!(
        transport
            .validate_compaction_window(&window, &foreign)
            .await,
        Err(ModelError::Authentication)
    ));
    transport.kind = ResponsesEndpointKind::Platform;
    assert!(matches!(
        transport.validate_compaction_window(&window, &source).await,
        Err(ModelError::UnsupportedCapability)
    ));
    transport.kind = ResponsesEndpointKind::ChatGpt;
    epoch.send(Uuid::new_v4()).unwrap();
    assert!(matches!(
        transport.validate_compaction_window(&window, &source).await,
        Err(ModelError::Authentication)
    ));
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 0);
    assert!(sink.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn window_validation_checks_live_lease_without_inference_or_replay_mutation() {
    use research_agent_core::context::checkpoint::ContextCheckpoint;
    for (other_account, revoked) in [(false, false), (true, false), (false, true)] {
        let (lease, epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account("window-account");
        let account = lease.scope_id();
        let (other, _other_epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account("other-account");
        let sink = Arc::new(Receipts::default());
        let mut transport = transport("http://127.0.0.1:1/responses".into(), sink.clone());
        transport.auth =
            Authentication::SyntheticLease(Arc::new(if other_account { other } else { lease }));
        if revoked {
            epoch.send(Uuid::new_v4()).unwrap();
        }
        let window = ContextCheckpoint {
            schema_version: 1,
            window_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            source_watermark: 0,
            previous_window_id: None,
            binding: compact_request().binding,
            credential_scope_id: Some(account),
            phase: CompactionPhase::Manual,
            output: CompactionOutput::RemoteOpaque(opaque()),
            retained: vec![],
            canonical_context: None,
        };
        let result = transport
            .validate_compaction_window(&window, &request())
            .await;
        if other_account || revoked {
            assert!(matches!(result, Err(ModelError::Authentication)));
        } else {
            result.unwrap();
        }
        assert_eq!(sink.prepares.load(Ordering::SeqCst), 0);
        assert!(sink.values.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn remote_candidate_preserves_payload_and_usage_without_promoting_continuation() {
    let sink = Arc::new(Receipts::default());
    let payload = opaque();
    let other = json!({"type":"function_call","id":"ignored","name":"never_execute"});
    let (url, peer) = server(ok(&events(&[other, payload.clone()], true)), false).await;
    let mut request = compact_request();
    request
        .source
        .tools
        .push(research_agent_core::model::ToolDefinition {
            namespace: None,
            name: "registered_read".into(),
            description: "Read frozen evidence".into(),
            parameters: json!({"type":"object"}),
        });
    let strategy = ResponsesV2CompactionStrategy::new(
        Arc::new(transport(url, sink.clone())),
        request.binding.clone(),
    )
    .unwrap();
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert!(candidate.output == CompactionOutput::RemoteOpaque(payload));
    assert_eq!(candidate.response_id.as_deref(), Some("compact1"));
    assert_eq!(candidate.usage.input_tokens, Some(80));
    assert_eq!(candidate.credential_scope_id, Some(Uuid::from_u128(1)));
    assert!(sink.values.lock().unwrap().is_empty());
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 1);
    let sent = peer.await.unwrap();
    assert_eq!(
        sent["input"].as_array().unwrap().last().unwrap(),
        &json!({"type":"compaction_trigger"})
    );
    assert_eq!(sent["input"][0]["content"][0]["text"], "hello");
    assert_eq!(sent["tools"][0]["name"], "registered_read");
}

#[tokio::test]
async fn invalid_remote_streams_never_create_a_candidate_or_receipt() {
    for (items, completed) in [
        (vec![], true),
        (vec![opaque(), opaque()], true),
        (vec![opaque()], false),
        (
            vec![json!({"type":"compaction","encrypted_content":123})],
            true,
        ),
    ] {
        let sink = Arc::new(Receipts::default());
        let content = events(&items, completed);
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
            content.len(),
            content
        );
        let (url, peer) = server(reply, false).await;
        let request = compact_request();
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(url, sink.clone())),
            request.binding.clone(),
        )
        .unwrap();
        assert!(prepare_compaction(&strategy, request).await.is_err());
        peer.await.unwrap();
        assert!(sink.values.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn ordinary_sampling_still_rejects_compaction_items() {
    let sink = Arc::new(Receipts::default());
    let (url, peer) = server(ok(&events(&[opaque()], true)), false).await;
    let transport = transport(url, sink.clone());
    let mut stream = transport.start(request()).await.unwrap();
    // A single HTTP chunk may include created and the invalid done item. The
    // stream can reject that chunk before exposing the queued Started event.
    let first = stream.next().await;
    if first.is_ok() {
        assert!(stream.next().await.is_err());
    }
    peer.await.unwrap();
    assert!(sink.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dropping_remote_candidate_future_closes_the_owned_http_response() {
    let sink = Arc::new(Receipts::default());
    let pending = "data: {\"type\":\"response.created\",\"response\":{\"id\":\"pending\",\"model\":\"fixture-model\"}}\n\n";
    let (url, peer) = server(ok(pending), true).await;
    let request = compact_request();
    let strategy = ResponsesV2CompactionStrategy::new(
        Arc::new(transport(url, sink.clone())),
        request.binding.clone(),
    )
    .unwrap();
    assert!(
        timeout(Duration::from_millis(100), strategy.compact(request))
            .await
            .is_err()
    );
    peer.await.unwrap();
    assert!(sink.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remote_candidate_honors_payload_budget_and_fixed_protocol_alias() {
    use research_agent_core::context::compaction_strategy::CompactionFailure;
    for bounded in [false, true] {
        let mut item = opaque();
        item["type"] = json!("compaction_summary");
        item["encrypted_content"] = json!("");
        let sink = Arc::new(Receipts::default());
        let (url, peer) = server(ok(&events(&[item.clone()], true)), false).await;
        let mut request = compact_request();
        if bounded {
            request.max_result_bytes = 1;
        }
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(url, sink.clone())),
            request.binding.clone(),
        )
        .unwrap();
        let result = prepare_compaction(&strategy, request).await;
        if bounded {
            assert!(matches!(result, Err(CompactionFailure::OutputBudget)));
        } else {
            assert!(result.unwrap().output == CompactionOutput::RemoteOpaque(item));
        }
        peer.await.unwrap();
        assert!(sink.values.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn changed_frozen_model_is_rejected_before_http_or_receipt_preparation() {
    use research_agent_core::context::compaction_strategy::CompactionFailure;
    let sink = Arc::new(Receipts::default());
    let mut request = compact_request();
    let strategy = ResponsesV2CompactionStrategy::new(
        Arc::new(transport(
            "http://127.0.0.1:1/responses".into(),
            sink.clone(),
        )),
        request.binding.clone(),
    )
    .unwrap();
    request.source.model.model_id = "different-model".into();
    assert!(matches!(
        prepare_compaction(&strategy, request).await,
        Err(CompactionFailure::Incompatible)
    ));
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 0);
    assert!(sink.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remote_candidate_uses_the_actual_lease_scope_instead_of_configuration() {
    let sink = Arc::new(Receipts::default());
    let (url, peer) = server(ok(&events(&[opaque()], true)), false).await;
    let (lease, _epoch) = crate::agent::authentication::tests::synthetic_lease();
    let actual_scope = lease.scope_id();
    let mut transport = transport(url, sink.clone());
    transport.config.credential_scope_id = Uuid::from_u128(987);
    assert_ne!(actual_scope, transport.config.credential_scope_id);
    transport.auth = Authentication::SyntheticLease(Arc::new(lease));
    let request = compact_request();
    let strategy =
        ResponsesV2CompactionStrategy::new(Arc::new(transport), request.binding.clone()).unwrap();
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert_eq!(candidate.credential_scope_id, Some(actual_scope));
    assert!(sink.values.lock().unwrap().is_empty());
    peer.await.unwrap();
}

#[tokio::test]
async fn missing_account_scope_is_rejected_before_input_preparation() {
    let sink = Arc::new(Receipts::default());
    let mut transport = transport("http://127.0.0.1:1/responses".into(), sink.clone());
    transport.config.credential_scope_id = Uuid::nil();
    let request = compact_request();
    let strategy =
        ResponsesV2CompactionStrategy::new(Arc::new(transport), request.binding.clone()).unwrap();
    assert!(matches!(
        prepare_compaction(&strategy, request).await,
        Err(
            research_agent_core::context::compaction_strategy::CompactionFailure::Model(
                ModelError::Authentication
            )
        )
    ));
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshed_remote_compaction_cannot_cross_accounts() {
    for changed in [false, true] {
        let (first, _first_epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account("first-account");
        let expected_scope = first.scope_id();
        let (second, _second_epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account(if changed {
                "other-account"
            } else {
                "first-account"
            });
        let credentials = Arc::new(CredentialSequence {
            leases: Mutex::new(VecDeque::from([first, second])),
            acquisitions: AtomicUsize::new(0),
        });
        let mut replies = vec![
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ];
        if !changed {
            replies.push(ok(&events(&[opaque()], true)));
        }
        let (url, peer) = sequential_server(replies).await;
        let sink = Arc::new(Receipts::default());
        let transport = synthetic_chatgpt_transport(url, credentials.clone(), sink.clone());
        let request = compact_request();
        let strategy =
            ResponsesV2CompactionStrategy::new(Arc::new(transport), request.binding.clone())
                .unwrap();
        let result = prepare_compaction(&strategy, request).await;
        if changed {
            assert!(matches!(
                result,
                Err(
                    research_agent_core::context::compaction_strategy::CompactionFailure::Model(
                        ModelError::Authentication
                    )
                )
            ));
        } else {
            assert_eq!(result.unwrap().credential_scope_id, Some(expected_scope));
        }
        assert_eq!(peer.await.unwrap().len(), if changed { 1 } else { 2 });
        assert_eq!(credentials.acquisitions.load(Ordering::SeqCst), 2);
        assert!(sink.values.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn installed_item_is_sent_as_protocol_state_once_across_normal_sampling() {
    use crate::agent::model::responses_continuation::ResponsesContinuation;
    let mut source = request();
    source
        .messages
        .insert(0, ModelMessage::System("current authority".into()));
    source.messages.insert(
        1,
        ModelMessage::User {
            content: vec![ContentPart::Text("retained user".into())],
            source_item_id: None,
        },
    );
    let owner = Arc::new(
        ResponsesContinuation::new(Uuid::new_v4(), source.model.clone(), 100_000).unwrap(),
    );
    let item = opaque();
    owner
        .bind_compaction_sync(Uuid::from_u128(1), &source.model, &item, 1)
        .unwrap();
    let (url, peer) = server(ok(&body()), false).await;
    let mut stream = transport(url, owner.clone())
        .start(source.clone())
        .await
        .unwrap();
    while stream.next().await.unwrap().is_some() {}
    let sent = peer.await.unwrap();
    assert_eq!(sent["input"][0]["role"], "developer");
    assert_eq!(sent["input"][1]["content"][0]["text"], "retained user");
    assert_eq!(sent["input"][2], item);
    assert_eq!(sent["input"][3]["content"][0]["text"], "hello");
    source.request_id = Uuid::new_v4();
    source.messages.push(ModelMessage::Assistant {
        phase: None,
        content: vec![ContentPart::Text("Hello".into())],
        tool_calls: vec![],
    });
    source.messages.push(ModelMessage::User {
        content: vec![ContentPart::Text("follow up".into())],
        source_item_id: None,
    });
    let (url, peer) = server(ok(&body()), false).await;
    let mut stream = transport(url, owner).start(source).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    let sent = peer.await.unwrap();
    assert_eq!(sent["input"][2], item);
    assert_eq!(
        sent["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "compaction")
            .count(),
        1
    );
}

#[tokio::test]
async fn sparse_v2_identity_and_late_served_alias_do_not_reject_a_complete_candidate() {
    for late_alias in [false, true] {
        let mut frames = vec![
            json!({"type":"response.created","response":{"id":"sparse-compact","model":"configured-body-alias"}}),
        ];
        if late_alias {
            frames.push(json!({"type":"response.in_progress","response":{"id":"sparse-compact","headers":{"OpenAI-Model":"served-model-alias"}}}));
        }
        let item = opaque();
        frames.push(json!({"type":"response.output_item.done","item":item}));
        frames.push(json!({"type":"response.completed","response":{"id":"sparse-compact"}}));
        let payload = frames
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let (url, peer) = server(ok(&payload), true).await;
        let sink = Arc::new(Receipts::default());
        let request = compact_request();
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(url, sink.clone())),
            request.binding.clone(),
        )
        .unwrap();
        let candidate = strategy.compact(request).await.unwrap();
        assert_eq!(candidate.response_id.as_deref(), Some("sparse-compact"));
        assert!(candidate.output == CompactionOutput::RemoteOpaque(item));
        assert!(sink.values.lock().unwrap().is_empty());
        peer.await.unwrap();
    }
}
