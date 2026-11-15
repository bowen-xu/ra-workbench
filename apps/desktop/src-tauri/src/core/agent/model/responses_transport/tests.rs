use super::*;
#[path = "tests/compaction_input.rs"]
mod compaction_input;
#[path = "tests/compaction_tests.rs"]
mod compaction_tests;
#[path = "tests/legacy_compaction.rs"]
mod legacy_compaction;
#[path = "tests/legacy_retry.rs"]
mod legacy_retry;
#[path = "tests/metadata.rs"]
mod metadata;
#[path = "tests/native_compaction.rs"]
mod native_compaction;
#[path = "tests/progress.rs"]
mod progress;
#[path = "tests/reasoning_summary.rs"]
mod reasoning_summary;
#[path = "tests/turn_state.rs"]
mod turn_state;
#[path = "tests/v2_retry.rs"]
mod v2_retry;
use crate::agent::authentication::{AuthenticationError, CredentialFuture};
use research_agent_core::model::{ContentPart, ModelMessage};
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

#[derive(Default)]
struct Receipts {
    values: Mutex<Vec<ResponsesReceipt>>,
    prepares: AtomicUsize,
    fail: bool,
}
impl ResponsesReceiptSink for Receipts {
    fn prepare_input(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(input) })
    }
    fn preserve(&self, receipt: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async move {
            if self.fail {
                return Err(ModelError::Unavailable);
            }
            self.values.lock().unwrap().push(receipt);
            Ok(())
        })
    }
}

struct CredentialSequence {
    leases: Mutex<VecDeque<ChatGptLease>>,
    acquisitions: AtomicUsize,
}

impl ChatGptCredentialSource for CredentialSequence {
    fn acquire(&self) -> CredentialFuture<'_> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            self.leases
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(AuthenticationError::Unavailable)
        })
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture-model".into(),
            revision: None,
        },
        messages: vec![ModelMessage::User {
            content: vec![ContentPart::Text("hello".into())],
            source_item_id: None,
        }],
        tools: vec![],
        max_output_tokens: 128,
        parallel_tool_calls: false,
        reasoning_effort: None,
    }
}
fn transport(endpoint: String, sink: Arc<dyn ResponsesReceiptSink>) -> ResponsesTransport {
    ResponsesTransport::build(
        ResponsesTransportConfig {
            credential_scope_id: Uuid::from_u128(1),
            provider_id: "fixture".into(),
            service_tier: None,
            timeout: Duration::from_secs(5),
            capabilities: ModelCapabilities {
                vision: true,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
        },
        endpoint,
        ResponsesEndpointKind::Platform,
        Authentication::Platform(HeaderValue::from_static("Bearer synthetic")),
        sink,
    )
    .unwrap()
}
fn body() -> String {
    let item = json!({"type":"message","id":"m1","role":"assistant","content":[{"type":"output_text","text":"Hello"}]});
    [json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture-model","status":"completed","output":[item]}})]
        .into_iter().map(|event|format!("data: {event}\n\n")).collect()
}
async fn server(reply: String, wait: bool) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            let (start, length) = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buffer[..n]);
                assert!(data.len() < 65536);
                if let Some(end) = data.windows(4).position(|x| x == b"\r\n\r\n") {
                    let headers = String::from_utf8(data[..end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while data.len() < start + length {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buffer[..n]);
            }
            let request = serde_json::from_slice(&data[start..start + length]).unwrap();
            socket.write_all(reply.as_bytes()).await.unwrap();
            if wait {
                assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
            }
            request
        })
        .await
        .unwrap()
    });
    (url, task)
}

async fn sequential_server(replies: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let (url, peer) = sequential_server_with_headers(replies).await;
    (url, tokio::spawn(async move { peer.await.unwrap().0 }))
}

async fn sequential_server_with_headers(
    replies: Vec<String>,
) -> (String, tokio::task::JoinHandle<(Vec<Value>, Vec<String>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let expected = replies.len();
        let mut requests = Vec::new();
        let mut request_headers = Vec::new();
        for (completed, reply) in replies.into_iter().enumerate() {
            // Bound each exchange, not the whole multi-turn database/fork scenario.
            // A completed reply gives the next request its own timeout budget.
            timeout(Duration::from_secs(5), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                let (start, length) = loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buffer[..n]);
                    assert!(data.len() < 65536);
                    if let Some(end) = data.windows(4).position(|x| x == b"\r\n\r\n") {
                        let headers = String::from_utf8(data[..end].to_vec()).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while data.len() < start + length {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buffer[..n]);
                }
                requests.push(serde_json::from_slice(&data[start..start + length]).unwrap());
                request_headers.push(String::from_utf8(data[..start - 4].to_vec()).unwrap());
                socket.write_all(reply.as_bytes()).await.unwrap();
            })
            .await
            .unwrap_or_else(|_| {
                panic!("mock HTTP sequence timed out: {completed}/{expected} replies completed")
            });
        }
        (requests, request_headers)
    });
    (url, task)
}

fn synthetic_chatgpt_transport(
    endpoint: String,
    credentials: Arc<dyn ChatGptCredentialSource>,
    receipts: Arc<dyn ResponsesReceiptSink>,
) -> ResponsesTransport {
    ResponsesTransport::build(
        ResponsesTransportConfig {
            credential_scope_id: Uuid::nil(),
            provider_id: "fixture".into(),
            service_tier: None,
            timeout: Duration::from_secs(5),
            capabilities: ModelCapabilities {
                vision: true,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
        },
        endpoint,
        ResponsesEndpointKind::ChatGpt,
        Authentication::SyntheticChatGpt(credentials),
        receipts,
    )
    .unwrap()
}

#[test]
fn namespace_catalog_capability_is_transport_specific() {
    let sink = Arc::new(Receipts::default());
    let platform = transport("http://fixture/responses".into(), sink.clone());
    let chatgpt = synthetic_chatgpt_transport(
        "http://fixture/responses".into(),
        Arc::new(CredentialSequence {
            leases: Mutex::new(VecDeque::new()),
            acquisitions: AtomicUsize::new(0),
        }),
        sink,
    );
    assert!(!platform.supports_namespace_tools());
    assert!(chatgpt.supports_namespace_tools());
    let mut request = request();
    request
        .tools
        .push(research_agent_core::model::ToolDefinition {
            namespace: Some("group".into()),
            name: "read".into(),
            description: "read".into(),
            parameters: json!({"type":"object"}),
        });
    assert_eq!(
        platform.request_body(&request, false),
        Err(ModelError::UnsupportedCapability)
    );
    assert!(chatgpt.request_body(&request, false).is_ok());
}
fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n",
        body.len()
    )
}

#[tokio::test]
async fn omitted_content_type_is_only_accepted_for_valid_chatgpt_streams() {
    for (chatgpt, header, valid_body, accepted) in [
        (true, "", true, true),
        (false, "", true, false),
        (true, "Content-Type: application/json\r\n", true, false),
        (true, "", false, false),
    ] {
        let content = if valid_body {
            body()
        } else {
            "data: {}\n\n".into()
        };
        let reply = format!(
            "HTTP/1.1 200 OK\r\n{header}Content-Length: {}\r\n\r\n{content}",
            content.len()
        );
        let (url, server) = server(reply, false).await;
        let receipts = Arc::new(Receipts::default());
        let mut transport = transport(url, receipts.clone());
        if chatgpt {
            transport.kind = ResponsesEndpointKind::ChatGpt;
        }
        let outcome = async {
            let mut stream = transport.start(request()).await?;
            while stream.next().await?.is_some() {}
            Ok::<_, ModelError>(())
        }
        .await;
        assert_eq!(outcome.is_ok(), accepted);
        assert!(receipts.values.lock().unwrap().is_empty());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn completion_preserves_native_output_and_releases_open_http_body() {
    let (url, server) = server(ok(&body()), true).await;
    let sink = Arc::new(Receipts::default());
    let transport = transport(url, sink.clone());
    let request = request();
    let mut stream = transport.start(request).await.unwrap();
    let mut text = String::new();
    let mut completed = false;
    let mut canonical = vec![];
    while let Some(event) = stream.next().await.unwrap() {
        match event {
            ModelStreamEvent::TextDelta(delta)
            | ModelStreamEvent::AssistantTextDelta { text: delta, .. } => text.push_str(&delta),
            ModelStreamEvent::Completed { .. } => completed = true,
            ModelStreamEvent::OutputItemDone(item) => canonical.push(item),
            _ => {}
        }
    }
    assert_eq!(text, "Hello");
    assert!(completed);
    assert_eq!(canonical.len(), 1);
    let item = &canonical[0];
    assert_eq!(item.native.as_ref().unwrap().item_id.as_deref(), Some("m1"));
    assert!(
        matches!(&item.normalized, Some(research_agent_core::model::ModelOutputItem::Message(message)) if message.text == "Hello")
    );
    assert!(sink.values.lock().unwrap().is_empty());
    let sent = server.await.unwrap();
    assert_eq!(sent["store"], false);
    assert_eq!(sent["max_output_tokens"], 128);
}

#[tokio::test]
async fn canonical_stream_does_not_write_a_second_response_receipt() {
    let (url, server) = server(ok(&body()), true).await;
    let sink = Arc::new(Receipts {
        fail: true,
        ..Default::default()
    });
    let transport = transport(url, sink.clone());
    let mut stream = transport.start(request()).await.unwrap();
    let mut accepted = false;
    let mut completed = false;
    while let Some(event) = stream.next().await.unwrap() {
        accepted |= matches!(event, ModelStreamEvent::OutputItemDone(_));
        completed |= matches!(event, ModelStreamEvent::Completed { .. });
    }
    assert!(accepted && completed);
    assert!(sink.values.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn eof_without_protocol_completion_is_not_success() {
    let payload = format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}})
    );
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let (url, server) = server(reply, false).await;
    let sink = Arc::new(Receipts::default());
    let transport = transport(url, sink.clone());
    let mut stream = transport.start(request()).await.unwrap();
    assert!(matches!(
        stream.next().await.unwrap(),
        Some(ModelStreamEvent::Started { .. })
    ));
    assert!(matches!(
        stream.next().await,
        Err(ModelError::IncompleteStream)
    ));
    assert!(sink.values.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn redirects_and_authentication_errors_are_not_followed_or_retried() {
    for (code, error) in [
        (302, ModelError::UnexpectedStatus(302)),
        (400, ModelError::UnexpectedStatus(400)),
        (422, ModelError::UnexpectedStatus(422)),
        (401, ModelError::HttpAuthentication(401)),
        (403, ModelError::HttpAuthentication(403)),
        (429, ModelError::HttpRateLimited(429)),
        (503, ModelError::UnexpectedStatus(503)),
    ] {
        let(url,server)=server(format!("HTTP/1.1 {code} Fixture\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\n\r\n"),false).await;
        let transport = transport(url, Arc::new(Receipts::default()));
        assert!(matches!(transport.start(request()).await,Err(actual) if actual==error));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn chatgpt_unauthorized_refreshes_once_without_repreparing_input() {
    let (first, _first_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let (second, _second_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let credentials = Arc::new(CredentialSequence {
        leases: Mutex::new(VecDeque::from([first, second])),
        acquisitions: AtomicUsize::new(0),
    });
    let replies = vec![
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ok(&body()),
    ];
    let (url, server) = sequential_server(replies).await;
    let receipts = Arc::new(Receipts::default());
    let transport = synthetic_chatgpt_transport(url, credentials.clone(), receipts.clone());

    let mut stream = transport.start(request()).await.unwrap();
    while stream.next().await.unwrap().is_some() {}

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(credentials.acquisitions.load(Ordering::SeqCst), 2);
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn chatgpt_unauthorized_rejects_a_refreshed_account_scope() {
    let (first, _first_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("first-account");
    let (second, _second_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("other-account");
    let credentials = Arc::new(CredentialSequence {
        leases: Mutex::new(VecDeque::from([first, second])),
        acquisitions: AtomicUsize::new(0),
    });
    let (url, server) = server(
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into(),
        false,
    )
    .await;
    let receipts = Arc::new(Receipts::default());
    let transport = synthetic_chatgpt_transport(url, credentials.clone(), receipts.clone());

    assert!(matches!(
        transport.start(request()).await,
        Err(ModelError::Authentication)
    ));
    server.await.unwrap();
    assert_eq!(credentials.acquisitions.load(Ordering::SeqCst), 2);
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn chatgpt_unauthorized_is_retried_at_most_once() {
    let (first, _first_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let (second, _second_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let credentials = Arc::new(CredentialSequence {
        leases: Mutex::new(VecDeque::from([first, second])),
        acquisitions: AtomicUsize::new(0),
    });
    let replies = vec![
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    ];
    let (url, server) = sequential_server(replies).await;
    let receipts = Arc::new(Receipts::default());
    let transport = synthetic_chatgpt_transport(url, credentials.clone(), receipts.clone());

    assert!(matches!(
        transport.start(request()).await,
        Err(ModelError::HttpAuthentication(403))
    ));
    assert_eq!(server.await.unwrap().len(), 2);
    assert_eq!(credentials.acquisitions.load(Ordering::SeqCst), 2);
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn canceled_next_future_keeps_lease_and_revocation_closes_pending_stream() {
    let payload = format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}})
    );
    let (url, server) = server(ok(&payload), true).await;
    let sink = Arc::new(Receipts::default());
    let mut transport = transport(url, sink.clone());
    let (lease, epoch) = crate::agent::authentication::tests::synthetic_lease();
    transport.auth = Authentication::SyntheticLease(Arc::new(lease));
    let mut stream = transport.start(request()).await.unwrap();
    assert!(matches!(
        stream.next().await.unwrap(),
        Some(ModelStreamEvent::Started { .. })
    ));
    let mut pending = stream.next();
    assert!(matches!(
        futures_util::poll!(&mut pending),
        std::task::Poll::Pending
    ));
    drop(pending);
    epoch.send_replace(Uuid::new_v4());
    assert!(matches!(
        timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap(),
        Err(ModelError::Authentication)
    ));
    assert!(sink.values.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn two_http_rounds_replay_reasoning_with_tool_result() {
    use super::super::responses_continuation::ResponsesContinuation;
    use research_agent_core::model::ToolDefinition;
    let mut first_request = request();
    first_request.reasoning_effort = Some("medium".into());
    first_request.tools.push(ToolDefinition {
        namespace: None,
        name: "read".into(),
        description: "Read".into(),
        parameters: json!({"type":"object"}),
    });
    let state = Arc::new(
        ResponsesContinuation::new(Uuid::new_v4(), first_request.model.clone(), 65536).unwrap(),
    );
    let reasoning = json!({"type":"reasoning","id":"reason1","encrypted_content":"opaque-http-fixture","summary":[]});
    let call = json!({"type":"function_call","id":"call-item","call_id":"call1","name":"read","arguments":"{ }"});
    let frames=[json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}}),
        json!({"type":"response.output_item.done","item":reasoning}),json!({"type":"response.output_item.done","item":call}),
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture-model","status":"completed","output":[reasoning,call]}})]
        .into_iter().map(|event|format!("data: {event}\n\n")).collect::<String>();
    let (url, first_server) = server(ok(&frames), true).await;
    let mut first = transport(url, state.clone());
    first.config.capabilities.reasoning_efforts = vec!["medium".into()];
    let mut stream = first.start(first_request.clone()).await.unwrap();
    let mut canonical = vec![];
    while let Some(event) = stream.next().await.unwrap() {
        if let ModelStreamEvent::OutputItemDone(item) = event {
            canonical.push(ModelMessage::CanonicalOutput(item));
        }
    }
    assert_eq!(canonical.len(), 2);
    assert_eq!(first_server.await.unwrap()["reasoning"]["effort"], "medium");
    let mut followup = first_request;
    followup.request_id = Uuid::new_v4();
    followup.messages.extend(canonical);
    followup.messages.push(ModelMessage::ToolResult {
        call_id: "call1".into(),
        content: "source text".into(),
        is_error: false,
    });
    let (url, second_server) = server(ok(&body()), true).await;
    let mut second = transport(url, state);
    second.config.capabilities.reasoning_efforts = vec!["medium".into()];
    let mut stream = second.start(followup).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    let sent = second_server.await.unwrap();
    assert_eq!(sent["reasoning"]["effort"], "medium");
    assert_eq!(sent["input"][1]["encrypted_content"], "opaque-http-fixture");
    assert_eq!(sent["input"][2]["arguments"], "{ }");
    assert_eq!(sent["input"][3]["call_id"], "call1");
    assert_eq!(sent["input"][3]["output"], "source text");
}

#[tokio::test]
async fn changed_credential_revision_is_rejected_before_http_connect() {
    use super::super::responses_continuation::ResponsesContinuation;
    let request = request();
    let state =
        Arc::new(ResponsesContinuation::new(Uuid::new_v4(), request.model.clone(), 65536).unwrap());
    state
        .prepare_input(Uuid::from_u128(1), request.model.clone(), vec![])
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut transport = transport(
        format!("http://{}/responses", listener.local_addr().unwrap()),
        state,
    );
    transport.config.credential_scope_id = Uuid::from_u128(2);
    assert!(matches!(
        transport.start(request).await,
        Err(ModelError::Authentication)
    ));
    let accept = listener.accept();
    tokio::pin!(accept);
    assert!(matches!(
        futures_util::poll!(&mut accept),
        std::task::Poll::Pending
    ));
}

#[tokio::test]
async fn frozen_fast_mode_uses_codex_priority_request_value() {
    for (tier, expected) in [
        (Some("fast"), Some("priority")),
        (Some("default"), Some("default")),
        (None, None),
    ] {
        let (url, server) = server(ok(&body()), false).await;
        let mut transport = transport(url, Arc::new(Receipts::default()));
        transport.config.provider_id = "openai".into();
        transport.config.service_tier = tier.map(str::to_owned);
        let mut request = request();
        request.model.provider_id = "openai".into();
        let mut stream = transport.start(request).await.unwrap();
        while stream.next().await.unwrap().is_some() {}
        let sent = server.await.unwrap();
        assert_eq!(sent.get("service_tier").and_then(Value::as_str), expected);
    }
}
