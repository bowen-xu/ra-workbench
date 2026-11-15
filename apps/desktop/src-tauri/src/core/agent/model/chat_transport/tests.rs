use super::*;
use research_agent_core::{
    model::{ContentPart, ModelMessage, ResponseAssembler, StreamLimits, ToolCall, ToolDefinition},
    protocol::ModelIdentity,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};

fn config() -> ChatTransportConfig {
    ChatTransportConfig {
        request_policy: Default::default(),
        provider_id: "fixture".into(),
        base_url: String::new(),
        timeout: Duration::from_secs(5),
        capabilities: ModelCapabilities {
            vision: true,
            tools: true,
            parallel_tools: true,
            reasoning_efforts: vec![],
        },
        output_token_parameter: OutputTokenParameter::MaxTokens,
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        captured_history: None,
        request_id: uuid::Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "chosen".into(),
            revision: None,
        },
        messages: vec![ModelMessage::User {
            content: vec![ContentPart::Text("read".into())],
            source_item_id: None,
        }],
        tools: vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read manuscript".into(),
            parameters: json!({"type":"object"}),
        }],
        max_output_tokens: 128,
        parallel_tool_calls: false,
        reasoning_effort: None,
    }
}
fn data(delta: Value, reason: Option<&str>) -> String {
    format!(
        "data: {}\r\n\r\n",
        json!({"id":"response","model":"actual","object":"chat.completion.chunk",
        "choices":[{"index":0,"delta":delta,"finish_reason":reason}]})
    )
}
fn response(body: &str) -> Vec<u8> {
    // Deliberately omit the terminating HTTP chunk: [DONE] owns completion.
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n",body.len(),body).into_bytes()
}
async fn server(reply: Vec<u8>, wait_for_close: bool) -> (String, JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/chat/completions", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 2048];
            let (headers, body_start, len) = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 64 * 1024);
                if let Some(index) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..index].to_vec()).unwrap();
                    let len = headers
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    break (headers, index + 4, len);
                }
            };
            while bytes.len() < body_start + len {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let body = serde_json::from_slice(&bytes[body_start..body_start + len]).unwrap();
            socket.write_all(&reply).await.unwrap();
            if wait_for_close {
                assert_eq!(
                    socket.read(&mut buffer).await.unwrap(),
                    0,
                    "client must release unfinished HTTP body"
                );
            }
            socket.shutdown().await.unwrap();
            (headers, body)
        })
        .await
        .expect("bounded fixture server")
    });
    (url, handle)
}
fn transport(url: String) -> ChatTransport {
    ChatTransport::build(
        config(),
        HeaderValue::from_static("Bearer synthetic-fixture"),
        url,
    )
    .unwrap()
}

#[tokio::test]
async fn actual_http_stream_preserves_tools_usage_and_request_history() {
    let body = format!(
        "{}{}data: {}\n\ndata: [DONE]\n\n",
        data(
            json!({"content":"中文","tool_calls":[{"index":0,"id":"new-call","type":"function","function":{"name":"read","arguments":"{"}}]}),
            None
        ),
        data(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"}"}}]}),
            Some("tool_calls")
        ),
        json!({"id":"response","object":"chat.completion.chunk","model":"actual","choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5}})
    );
    let (url, server) = server(response(&body), true).await;
    let mut transport = transport(url);
    transport.config.capabilities.reasoning_efforts = vec!["medium".into()];
    assert!(transport.authorization.is_sensitive());
    let mut input = request();
    input.reasoning_effort = Some("medium".into());
    input.messages.push(ModelMessage::Assistant {
        phase: None,
        content: vec![],
        tool_calls: vec![ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "old-call".into(),
            name: "read".into(),
            arguments: json!({"path":"a.tex"}),
        }],
    });
    input.messages.push(ModelMessage::ToolResult {
        call_id: "old-call".into(),
        content: "missing".into(),
        is_error: true,
    });
    input.messages.push(ModelMessage::User {
        content: vec![ContentPart::Image {
            media_type: "image/png".into(),
            bytes: vec![1, 2, 3].into(),
        }],
        source_item_id: None,
    });
    let mut stream = transport.start(input).await.unwrap();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 1024,
        argument_bytes: 1024,
        tool_calls: 2,
    })
    .unwrap();
    while let Some(event) = stream.next().await.unwrap() {
        assembler.push(event).unwrap();
    }
    let result = assembler.finish().unwrap();
    assert_eq!(result.text, "中文");
    assert_eq!(result.tool_calls[0].call_id, "new-call");
    assert_eq!(result.usage.input_tokens, Some(20));
    assert_eq!(result.actual_model.model_id, "actual");
    let (headers, body) = server.await.unwrap();
    assert!(headers.starts_with("POST /chat/completions HTTP/1.1"));
    assert!(
        headers.to_lowercase().contains(
            "authorization: Bearer synthetic-fixture"
                .to_lowercase()
                .as_str()
        )
    );
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["reasoning_effort"], "medium");
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(
        body["messages"][1]["tool_calls"][0]["function"]["arguments"],
        "{\"path\":\"a.tex\"}"
    );
    assert_eq!(body["messages"][2]["tool_call_id"], "old-call");
    let error: Value =
        serde_json::from_str(body["messages"][2]["content"].as_str().unwrap()).unwrap();
    assert_eq!(error["is_error"], true);
    assert_eq!(
        body["messages"][3]["content"][0]["image_url"]["url"],
        "data:image/png;base64,AQID"
    );
}

#[tokio::test]
async fn dropping_stream_closes_live_response_without_detached_reader() {
    let (url, server) = server(response(&data(json!({"content":"partial"}), None)), true).await;
    let transport = transport(url);
    let mut stream = transport.start(request()).await.unwrap();
    assert!(matches!(
        stream.next().await.unwrap(),
        Some(ModelStreamEvent::Started { .. })
    ));
    drop(stream);
    server.await.unwrap();
}

#[tokio::test]
async fn clean_http_eof_without_protocol_done_is_incomplete() {
    let body = data(json!({"content":"partial"}), Some("stop"));
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let (url, server) = server(reply.into_bytes(), false).await;
    let transport = transport(url);
    let mut stream = transport.start(request()).await.unwrap();
    loop {
        match stream.next().await {
            Ok(Some(_)) => {}
            Err(ModelError::IncompleteStream) => break,
            _ => panic!("must reject missing DONE"),
        }
    }
    assert!(matches!(
        stream.next().await,
        Err(ModelError::InvalidStream)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn http_errors_and_redirects_are_not_retried_or_exposed_as_content() {
    for (status, error) in [
        (401, ModelError::HttpAuthentication(401)),
        (403, ModelError::HttpAuthentication(403)),
        (429, ModelError::HttpRateLimited(429)),
        (503, ModelError::UnexpectedStatus(503)),
        (302, ModelError::UnexpectedStatus(302)),
    ] {
        let reply = format!(
            "HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:9/forbidden\r\nContent-Length: 0\r\n\r\n"
        );
        let (url, server) = server(reply.into_bytes(), false).await;
        assert!(matches!(transport(url).start(request()).await,Err(actual) if actual==error));
        server.await.unwrap();
    }
}

#[test]
fn public_constructor_keeps_existing_provider_endpoint_boundary() {
    for url in [
        "http://api.deepseek.com",
        "https://api.deepseek.com.evil.invalid",
        "https://secret@api.deepseek.com",
        "https://api.deepseek.com/?token=x",
    ] {
        let mut config = config();
        config.provider_id = "deepseek".into();
        config.base_url = url.into();
        assert!(matches!(
            ChatTransport::new(config, HeaderValue::from_static("Bearer fixture")),
            Err(ModelError::InvalidRequest)
        ));
    }
}

#[tokio::test]
async fn cancel_before_headers_releases_pending_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = transport(format!(
        "http://{}/chat/completions",
        listener.local_addr().unwrap()
    ));
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 2048];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            accepted_tx.send(()).unwrap();
            while socket.read(&mut bytes).await.unwrap() > 0 {}
        })
        .await
        .expect("canceled request must close socket");
    });
    let mut pending = transport.start(request());
    tokio::select! {
        result = &mut pending => { assert!(result.is_ok(), "request must still await headers"); panic!("unexpected response"); },
        result = accepted_rx => result.unwrap(),
    }
    drop(pending);
    server.await.unwrap();
}

#[tokio::test]
async fn non_sse_success_response_is_rejected_before_consuming_content() {
    let (url, server) = server(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n".to_vec(),
        false,
    )
    .await;
    assert!(matches!(
        transport(url).start(request()).await,
        Err(ModelError::InvalidStream)
    ));
    server.await.unwrap();
}

#[test]
fn request_capabilities_and_wire_token_parameter_are_explicit() {
    let mut input = request();
    let capabilities = config().capabilities;
    let body = chat_request::encode(
        &input,
        capabilities.clone(),
        OutputTokenParameter::MaxCompletionTokens,
    )
    .unwrap();
    assert_eq!(body["max_completion_tokens"], 128);
    assert!(body.get("max_tokens").is_none());
    input.messages.push(ModelMessage::Assistant {
        phase: None,
        content: vec![ContentPart::Image {
            media_type: "image/png".into(),
            bytes: vec![1].into(),
        }],
        tool_calls: vec![],
    });
    assert!(matches!(
        chat_request::encode(&input, capabilities, OutputTokenParameter::MaxTokens),
        Err(ModelError::UnsupportedCapability)
    ));
}

#[test]
fn reasoning_is_encoded_only_when_explicit_and_supported() {
    let mut input = request();
    let mut profile = config().capabilities;
    let body =
        chat_request::encode(&input, profile.clone(), OutputTokenParameter::MaxTokens).unwrap();
    assert!(body.get("reasoning_effort").is_none());
    input.reasoning_effort = Some("medium".into());
    assert!(matches!(
        chat_request::encode(&input, profile.clone(), OutputTokenParameter::MaxTokens),
        Err(ModelError::UnsupportedCapability)
    ));
    profile.reasoning_efforts = vec!["medium".into()];
    let body = chat_request::encode(&input, profile, OutputTokenParameter::MaxTokens).unwrap();
    assert_eq!(body["reasoning_effort"], "medium");
}

#[tokio::test]
async fn explicit_deepseek_policy_reaches_the_actual_http_request() {
    let body = format!(
        "{}data: [DONE]\r\n\r\n",
        data(json!({"content":"answer"}), Some("stop"))
    );
    let (url, server) = server(response(&body), true).await;
    let mut transport = transport(url);
    transport.config.provider_id = "deepseek".into();
    transport.config.request_policy = super::super::ChatRequestPolicy::DeepSeekNonThinking;
    let mut input = request();
    input.model.provider_id = "deepseek".into();
    let mut stream = transport.start(input).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    drop(stream);
    let (_, captured) = server.await.unwrap();
    assert_eq!(captured["thinking"], json!({"type":"disabled"}));
    assert!(captured.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn unsupported_developer_context_is_rejected_during_window_validation_without_io() {
    use research_agent_core::context::{checkpoint::*, compaction_strategy::*};
    let transport = transport("https://fixture.invalid/chat/completions".into());
    let mut request = request();
    request.messages = vec![ModelMessage::Developer("trusted guidance".into())];
    let window = ContextCheckpoint {
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
            messages: store_messages(&request.messages).unwrap(),
        }),
    };
    window.validate(8192).unwrap();
    assert_eq!(
        transport
            .validate_compaction_window(&window, &request)
            .await,
        Err(ModelError::UnsupportedCapability)
    );
    assert_eq!(
        chat_request::encode(
            &request,
            transport.capabilities(),
            OutputTokenParameter::MaxTokens
        ),
        Err(ModelError::UnsupportedCapability)
    );
}

#[path = "developer_role_tests.rs"]
mod developer_role;

#[path = "native_history_tests.rs"]
mod native_history;
