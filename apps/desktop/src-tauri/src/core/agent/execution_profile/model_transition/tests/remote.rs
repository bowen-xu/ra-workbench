//! Actual HTTP wire tests with synthetic responses and credentials only.
use super::*;
use crate::agent::model::responses_transport::{
    ResponsesReceipt, ResponsesReceiptSink, ResponsesTransport, ResponsesTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Receipts;
impl ResponsesReceiptSink for Receipts {
    fn prepare_input(
        &self,
        _: uuid::Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async { Ok(input) })
    }
    fn preserve(&self, _: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn old_remote_model_generates_both_contracts_without_changing_account_or_output() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        let mut old = policy("old", Some(2000));
        old.binding.model.provider_id = "openai".into();
        old.binding = native.binding(old.binding.model.clone());
        old.authentication = ContextAuthentication::CodexBackend;
        let mut current = old.clone();
        current.binding.model.model_id = "new".into();
        assert!(allows_handoff(&old, &current));
        let item =
            json!({"type":"compaction","id":"cmp_synthetic","encrypted_content":"synthetic-state"});
        let output = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            CompactionOutput::RemoteOpaque(item.clone())
        } else {
            CompactionOutput::RemoteWindow(vec![
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"retained"}]}),
                item.clone(),
            ])
        };
        let response = if let CompactionOutput::RemoteWindow(window) = &output {
            json!({"model":"old","output":window}).to_string()
        } else {
            [json!({"type":"response.created","response":{"id":"r_old","model":"old"}}),
             json!({"type":"response.output_item.done","output_index":0,"item":item}),
             json!({"type":"response.completed","response":{"id":"r_old","model":"old","status":"completed","output":[item]}})]
             .into_iter().map(|e| format!("data: {e}\n\n")).collect()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mime = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            "text/event-stream"
        } else {
            "application/json"
        };
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let (start, size) = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + size {
                        break (end + 4, size);
                    }
                }
            };
            let body: Value = serde_json::from_slice(&bytes[start..start + size]).unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            body
        });
        let account = uuid::Uuid::new_v4();
        let transport = Arc::new(
            ResponsesTransport::synthetic_platform(
                ResponsesTransportConfig {
                    credential_scope_id: account,
                    provider_id: "openai".into(),
                    service_tier: None,
                    timeout: Duration::from_secs(3),
                    capabilities: ModelCapabilities {
                        vision: false,
                        tools: true,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                },
                &base,
                Arc::new(Receipts),
            )
            .unwrap(),
        );
        let strategy = native
            .responses_strategy(transport.clone(), old.binding.model.clone())
            .unwrap();
        let handoff = PreviousModelStrategy::compose(
            strategy,
            transport,
            current.binding.clone(),
            None,
            64,
            true,
        )
        .unwrap();
        assert_eq!(handoff.execution_model(), old.binding.model);
        for error in [
            ModelError::InvalidRequest,
            ModelError::UnexpectedStatus(422),
            ModelError::ContextWindowExceeded,
            ModelError::UsageLimitReached,
            ModelError::ServerOverloaded,
            ModelError::InternalServerError,
        ] {
            assert!(handoff.allows_current_model_fallback(&error));
        }
        for error in [
            ModelError::Authentication,
            ModelError::Canceled,
            ModelError::Unavailable,
            ModelError::RateLimited,
            ModelError::InvalidStream,
            ModelError::UnsupportedCapability,
        ] {
            assert!(!handoff.allows_current_model_fallback(&error));
        }
        assert!(
            handoff.retention_policy()
                == Some(
                    if native == NativeCompactionPolicy::ChatGptResponsesFullWindow {
                        CompactionRetention::ProviderWindow
                    } else {
                        CompactionRetention::RecentUserMessages
                    }
                )
        );
        let attempt = uuid::Uuid::new_v4();
        let request = CompactionRequest {
            operation_id: attempt,
            conversation_id: uuid::Uuid::new_v4(),
            turn_id: uuid::Uuid::new_v4(),
            history_revision: 1,
            phase: CompactionPhase::PreTurn,
            binding: current.binding.clone(),
            max_result_bytes: 8192,
            source: ModelRequest {
                captured_history: None,
                request_id: attempt,
                model: current.binding.model.clone(),
                messages: vec![ModelMessage::User {
                    content: vec![ContentPart::Text("historical evidence".into())],
                    source_item_id: None,
                }],
                tools: vec![],
                max_output_tokens: 128,
                parallel_tool_calls: false,
                reasoning_effort: None,
            },
        };
        let candidate = prepare_compaction(&handoff, request).await.unwrap();
        let sent = peer.await.unwrap();
        assert_eq!(sent["model"], "old");
        if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            assert_eq!(sent["max_output_tokens"], 64);
        }
        assert!(sent.to_string().contains("historical evidence"));
        assert_eq!(candidate.binding, current.binding);
        assert_eq!(candidate.credential_scope_id, Some(account));
        assert!(candidate.output == output);
    }
}
