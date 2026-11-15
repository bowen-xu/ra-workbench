use super::{compaction_tests::compact_request, *};
use research_agent_core::context::compaction_strategy::{
    CompactionFailure, CompactionKind, CompactionOutput, CompactionStrategy, prepare_compaction,
};

fn json_reply(value: &Value, status: u16) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn window() -> Value {
    json!([
        {"type":"message","role":"user","content":[{"type":"input_text","text":"retained evidence"}]},
        {"type":"compaction","encrypted_content":"opaque synthetic","provider_metadata":{"version":2}},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"retained answer"}]},
        {"type":"provider_future_item","value":{"opaque":true}}
    ])
}

#[cfg(all(debug_assertions, feature = "development-data"))]
#[tokio::test]
async fn legacy_preview_wire_replays_same_account_history_after_model_switch() {
    use crate::agent::model::responses_native::{NativeScope, capture};
    struct JsonWire(Mutex<Vec<Value>>);
    impl PreviewResponsesWire for JsonWire {
        fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
            let body: Value =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            self.0.lock().unwrap().push(body);
            Box::pin(async {
                Ok(reqwest::Response::from(
                    hyper::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(reqwest::Body::from(r#"{"output":[]}"#))
                        .unwrap(),
                ))
            })
        }
    }
    let account = Uuid::new_v4();
    let wire = Arc::new(JsonWire(Mutex::new(vec![])));
    let sink = Arc::new(Receipts::default());
    let mut source = request();
    source.model.provider_id = "openai".into();
    source.model.model_id = "new-model".into();
    let old = ModelIdentity {
        model_id: "old-model".into(),
        ..source.model.clone()
    };
    let raw = json!({"type":"function_call","id":"fc","call_id":"call","name":"read","namespace":"mcp__fixture","arguments":"{}"});
    let item = capture(
        &raw,
        &NativeScope {
            model: old,
            account_scope: account,
        },
        None,
        None,
    )
    .unwrap();
    source
        .messages
        .push(ModelMessage::CanonicalOutput(Box::new(item)));
    source.messages.push(ModelMessage::ToolResult {
        call_id: "call".into(),
        content: "synthetic".into(),
        is_error: false,
    });
    let native = Arc::new(
        ResponsesTransport::preview(
            ResponsesTransportConfig {
                credential_scope_id: account,
                provider_id: "openai".into(),
                timeout: Duration::from_secs(5),
                service_tier: None,
                capabilities: ModelCapabilities {
                    vision: true,
                    tools: true,
                    parallel_tools: false,
                    reasoning_efforts: vec![],
                },
            },
            wire.clone(),
            sink,
        )
        .unwrap(),
    );
    let mut compact = compact_request();
    compact.source = source;
    compact.binding.model = compact.source.model.clone();
    compact.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(native, compact.binding.clone()).unwrap();
    let mut foreign = compact.clone();
    let ModelMessage::CanonicalOutput(item) = &mut foreign.source.messages[1] else {
        unreachable!()
    };
    item.native.as_mut().unwrap().account_scope = Some(Uuid::new_v4());
    assert!(prepare_compaction(&strategy, foreign).await.is_err());
    assert!(wire.0.lock().unwrap().is_empty());
    let result = prepare_compaction(&strategy, compact).await.unwrap();
    assert!(result.output == CompactionOutput::RemoteWindow(vec![]));
    let bodies = wire.0.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call"
                && item["namespace"] == "mcp__fixture"
                && item["arguments"] == "{}")
    );
}

#[tokio::test]
async fn legacy_all_filtered_window_is_an_empty_candidate_without_publishing_receipt() {
    let payload = json!({"output":[
        {"type":"message","role":"developer","content":[{"type":"input_text","text":"stale"}]},
        {"type":"future_provider_item","value":"unknown"}
    ]});
    let (url, peer) = server(json_reply(&payload, 200), false).await;
    let receipts = Arc::new(Receipts::default());
    let mut request = compact_request();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url, receipts.clone())),
        request.binding.clone(),
    )
    .unwrap();
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert!(candidate.output == CompactionOutput::RemoteWindow(vec![]));
    assert_eq!(candidate.credential_scope_id, Some(Uuid::from_u128(1)));
    peer.await.unwrap();
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn installed_full_window_validation_checks_account_and_model_without_io() {
    use research_agent_core::context::checkpoint::ContextCheckpoint;
    let receipts = Arc::new(Receipts::default());
    let native = transport("http://127.0.0.1:1/responses".into(), receipts.clone());
    let source = request();
    let mut binding = compact_request().binding;
    binding.kind = CompactionKind::RemoteWindow;
    let mut checkpoint = ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        source_watermark: 0,
        previous_window_id: None,
        binding,
        credential_scope_id: Some(Uuid::from_u128(1)),
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        output: CompactionOutput::RemoteWindow(window().as_array().unwrap().clone()),
        retained: vec![],
        canonical_context: None,
    };
    assert!(native.supports_compaction_kind(CompactionKind::RemoteWindow));
    native
        .validate_compaction_window(&checkpoint, &source)
        .await
        .unwrap();
    checkpoint.output = CompactionOutput::RemoteWindow(vec![]);
    native
        .validate_compaction_window(&checkpoint, &source)
        .await
        .unwrap();
    checkpoint.credential_scope_id = Some(Uuid::from_u128(2));
    assert_eq!(
        native
            .validate_compaction_window(&checkpoint, &source)
            .await,
        Err(ModelError::Authentication)
    );
    checkpoint.credential_scope_id = Some(Uuid::from_u128(1));
    checkpoint.binding.model.model_id = "different-model".into();
    assert_eq!(
        native
            .validate_compaction_window(&checkpoint, &source)
            .await,
        Err(ModelError::UnsupportedCapability)
    );
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 0);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dropping_legacy_candidate_closes_incomplete_json_request_without_promotion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let ready = Arc::new(tokio::sync::Notify::new());
    let notified = ready.clone();
    let peer = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 65536);
                if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    assert!(headers.starts_with("POST /responses/compact HTTP/1.1\r\n"));
                    let length: usize = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap().parse().unwrap();
                    if bytes.len() >= end + 4 + length { break; }
                }
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{\"output\":").await.unwrap();
            notified.notify_one();
            match socket.read(&mut buffer).await {
                Ok(0) => {},
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
                other => panic!("candidate connection remained open: {other:?}"),
            }
        }).await.unwrap();
    });
    let receipts = Arc::new(Receipts::default());
    let mut request = compact_request();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url, receipts.clone())),
        request.binding.clone(),
    )
    .unwrap();
    {
        let candidate = prepare_compaction(&strategy, request);
        tokio::pin!(candidate);
        tokio::select! {
            _ = ready.notified() => {},
            _ = &mut candidate => panic!("unexpected completion before cancellation"),
        }
    }
    peer.await.unwrap();
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_json_candidate_filters_future_items_without_v2_trigger_or_replay_receipt() {
    let payload = json!({"model":"served-model-alias","output":window(), "usage":{"input_tokens":900,"output_tokens":12}});
    let (url, peer) = server(json_reply(&payload, 200), false).await;
    let receipts = Arc::new(Receipts::default());
    let mut native = transport(url, receipts.clone());
    native.config.provider_id = "openai".into();
    native.config.service_tier = Some("fast".into());
    let mut request = compact_request();
    request.source.model.provider_id = "openai".into();
    request.binding.model = request.source.model.clone();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy =
        ResponsesLegacyCompactionStrategy::new(Arc::new(native), request.binding.clone()).unwrap();
    assert!(matches!(
        strategy.retention_policy(),
        Some(
            research_agent_core::context::compaction_strategy::CompactionRetention::ProviderWindow
        )
    ));
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert!(
        candidate.output
            == CompactionOutput::RemoteWindow(window().as_array().unwrap()[..3].to_vec())
    );
    assert_eq!(candidate.response_id, None);
    assert_eq!(candidate.usage.input_tokens, Some(900));
    assert_eq!(candidate.usage.output_tokens, Some(12));
    assert_eq!(candidate.credential_scope_id, Some(Uuid::from_u128(1)));
    let sent = peer.await.unwrap();
    assert_eq!(sent["model"], "fixture-model");
    for key in [
        "stream",
        "store",
        "include",
        "max_output_tokens",
        "service_tier",
    ] {
        assert!(sent.get(key).is_none(), "unexpected {key}");
    }
    assert!(!sent.to_string().contains("compaction_trigger"));
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_candidate_rejects_malformed_windows_and_invalid_model_metadata() {
    for payload in [
        json!({}),
        json!({"output":[{}]}),
        json!({"output":["opaque"]}),
        json!({"output":[{"type":"compaction"}]}),
        json!({"output":[{"type":"message","role":"assistant","content":null}]}),
        json!({"output":[{"type":"message","role":"developer","content":null}]}),
        json!({"output":[{"type":"agent_message","content":[]}]}),
        json!({"output":[{"type":"reasoning","encrypted_content":"opaque"}]}),
        json!({"output":[{"type":"function_call","name":"read","arguments":{},"call_id":"call"}]}),
        json!({"output":[{"type":"function_call_output","call_id":"call","output":null}]}),
        json!({"output":window(),"model":13}),
        json!({"output":window(),"model":""}),
        json!({"output":window(),"id":13}),
        json!({"output":window(),"usage":{"input_tokens":-1,"output_tokens":1}}),
    ] {
        let (url, peer) = server(json_reply(&payload, 200), false).await;
        let mut request = compact_request();
        request.binding.kind = CompactionKind::RemoteWindow;
        let strategy = ResponsesLegacyCompactionStrategy::new(
            Arc::new(transport(url, Arc::new(Receipts::default()))),
            request.binding.clone(),
        )
        .unwrap();
        assert!(prepare_compaction(&strategy, request).await.is_err());
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn legacy_candidate_has_bounded_body_and_preserves_http_error_categories() {
    for (status, budget, expected) in [
        (401, 4096, ModelError::HttpAuthentication(401)),
        (429, 4096, ModelError::HttpRateLimited(429)),
        (503, 4096, ModelError::UnexpectedStatus(503)),
        (422, 4096, ModelError::UnexpectedStatus(422)),
        (200, 32, ModelError::BudgetExceeded),
    ] {
        let (url, peer) = server(json_reply(&json!({"output":window()}), status), false).await;
        let mut request = compact_request();
        request.binding.kind = CompactionKind::RemoteWindow;
        request.max_result_bytes = budget;
        let strategy = ResponsesLegacyCompactionStrategy::new(
            Arc::new(transport(url, Arc::new(Receipts::default()))),
            request.binding.clone(),
        )
        .unwrap();
        assert_eq!(
            prepare_compaction(&strategy, request).await.err(),
            Some(CompactionFailure::Model(expected))
        );
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn legacy_candidate_does_not_parse_sse_as_a_json_window() {
    let (url, peer) = server(ok(&body()), false).await;
    let mut request = compact_request();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url, Arc::new(Receipts::default()))),
        request.binding.clone(),
    )
    .unwrap();
    assert_eq!(
        prepare_compaction(&strategy, request).await.err(),
        Some(CompactionFailure::Model(ModelError::InvalidStream))
    );
    peer.await.unwrap();
}

#[tokio::test]
async fn legacy_auth_refresh_requires_same_scope_and_never_reprepares_input() {
    for (second_account, final_status) in [
        ("same-account", 200),
        ("same-account", 403),
        ("other-account", 200),
    ] {
        let (first, _first_epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
        let (second, _second_epoch) =
            crate::agent::authentication::tests::synthetic_lease_for_account(second_account);
        let credentials = Arc::new(CredentialSequence {
            leases: Mutex::new(VecDeque::from([first, second])),
            acquisitions: AtomicUsize::new(0),
        });
        let mut replies = vec![json_reply(&json!({}), 401)];
        if second_account == "same-account" {
            replies.push(json_reply(&json!({"output":window()}), final_status));
        }
        let (url, peer) = sequential_server(replies).await;
        let receipts = Arc::new(Receipts::default());
        let transport = synthetic_chatgpt_transport(url, credentials.clone(), receipts.clone());
        let mut request = compact_request();
        request.binding.kind = CompactionKind::RemoteWindow;
        let strategy =
            ResponsesLegacyCompactionStrategy::new(Arc::new(transport), request.binding.clone())
                .unwrap();
        let result = prepare_compaction(&strategy, request).await;
        if second_account == "same-account" && final_status == 200 {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.err(),
                Some(CompactionFailure::Model(
                    if second_account == "same-account" {
                        ModelError::HttpAuthentication(final_status)
                    } else {
                        ModelError::Authentication
                    }
                ))
            );
        }
        let sent = peer.await.unwrap();
        if sent.len() == 2 {
            assert_eq!(sent[0], sent[1]);
        }
        assert_eq!(credentials.acquisitions.load(Ordering::SeqCst), 2);
        assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
        assert!(receipts.values.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn legacy_empty_input_returns_bound_empty_window_without_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let receipts = Arc::new(Receipts::default());
    let mut request = compact_request();
    request.source.messages.clear();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(
            format!("http://{}/responses", listener.local_addr().unwrap()),
            receipts.clone(),
        )),
        request.binding.clone(),
    )
    .unwrap();
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert!(candidate.output == CompactionOutput::RemoteWindow(vec![]));
    assert_eq!(candidate.credential_scope_id, Some(Uuid::from_u128(1)));
    assert_eq!(candidate.response_id, None);
    assert_eq!(
        candidate.usage,
        research_agent_core::model::ModelUsage::default()
    );
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    assert!(receipts.values.lock().unwrap().is_empty());
}

struct InstalledWindowInput;
impl ResponsesReceiptSink for InstalledWindowInput {
    fn prepare_input(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move {
            assert!(input.is_empty());
            Ok(vec![
                json!({"type":"compaction","encrypted_content":"installed"}),
            ])
        })
    }
    fn preserve(&self, _: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async { panic!("compact cannot publish a sampling receipt") })
    }
}

#[tokio::test]
async fn legacy_empty_source_still_compacts_expanded_installed_history() {
    let (url, peer) = server(json_reply(&json!({"output": []}), 200), false).await;
    let mut request = compact_request();
    request.source.messages.clear();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url, Arc::new(InstalledWindowInput))),
        request.binding.clone(),
    )
    .unwrap();
    let candidate = prepare_compaction(&strategy, request).await.unwrap();
    assert!(candidate.output == CompactionOutput::RemoteWindow(vec![]));
    let sent = peer.await.unwrap();
    assert_eq!(
        sent["input"],
        json!([{"type":"compaction","encrypted_content":"installed"}])
    );
}

#[tokio::test]
async fn legacy_empty_input_keeps_request_and_account_validation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let mut request = compact_request();
    request.source.messages.clear();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url.clone(), Arc::new(Receipts::default()))),
        request.binding.clone(),
    )
    .unwrap();
    let mut invalid = request.clone();
    invalid.source.max_output_tokens = 0;
    assert_eq!(
        prepare_compaction(&strategy, invalid).await.err(),
        Some(CompactionFailure::Model(ModelError::InvalidRequest))
    );
    let mut native = transport(url, Arc::new(Receipts::default()));
    native.config.credential_scope_id = Uuid::nil();
    let strategy =
        ResponsesLegacyCompactionStrategy::new(Arc::new(native), request.binding.clone()).unwrap();
    assert_eq!(
        prepare_compaction(&strategy, request).await.err(),
        Some(CompactionFailure::Model(ModelError::Authentication))
    );
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_empty_candidate_enforces_persisted_budget_for_direct_callers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut request = compact_request();
    request.source.messages.clear();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(
            format!("http://{}/responses", listener.local_addr().unwrap()),
            Arc::new(Receipts::default()),
        )),
        request.binding.clone(),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&CompactionOutput::RemoteWindow(vec![]))
        .unwrap()
        .len();
    request.max_result_bytes = bytes - 1;
    assert_eq!(
        strategy.compact(request.clone()).await.err(),
        Some(CompactionFailure::OutputBudget)
    );
    request.max_result_bytes = bytes;
    assert!(strategy.compact(request).await.is_ok());
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
