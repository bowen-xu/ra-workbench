//! Uses the parent synthetic HTTP fixture. Both socket ends run in the same
//! test process/network namespace.
use super::*;
use crate::agent::model::responses_native::{NativeScope, capture};
use research_agent_core::model::{CompletedModelItem, ModelOutputItem};

fn captured(original: Value) -> CompletedModelItem {
    capture(&original, &NativeScope::fixture(), None, None).unwrap()
}

fn message(part: Value) -> CompletedModelItem {
    captured(json!({"type":"message","id":"message","role":"assistant",
        "phase":"commentary","content":[{"type":"output_text","text":"readable"},part]}))
}

#[tokio::test]
async fn actual_http_preserves_proven_text_and_adjacent_raw_tool_intents() {
    let body = format!(
        "{}data: [DONE]\n\n",
        data(json!({"content":"ok"}), Some("stop"))
    );
    let (url, server) = server(response(&body), false).await;
    let mut input = request();
    input
        .messages
        .push(ModelMessage::CanonicalOutput(Box::new(captured(json!({
            "type":"message","id":"message","role":"assistant","phase":"commentary",
            "status":"completed","content":[
                {"type":"input_text","text":"中文\u{0000}","annotations":[]},
                {"type":"output_text","text":"é🙂","logprobs":[]}
            ]
        })))));
    let raw = ["  { invalid JSON\n", " {\"path\" : \"a.tex\" } "];
    for (index, raw) in raw.iter().enumerate() {
        input
            .messages
            .push(ModelMessage::CanonicalOutput(Box::new(captured(json!({
                "type":"function_call","id":format!("tool-{index}"),"status":"completed",
                "call_id":format!("call-{index}"),"name":"read","arguments":raw
            })))));
    }
    for index in 0..2 {
        input.messages.push(ModelMessage::ToolResult {
            call_id: format!("call-{index}"),
            content: "missing".into(),
            is_error: index == 0,
        });
    }
    let transport = transport(url);
    let mut stream = transport.start(input).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    drop(stream);
    let (_, sent) = server.await.unwrap();
    assert_eq!(
        sent["messages"][1],
        json!({"role":"assistant","content":"中文\u{0000}é🙂"})
    );
    let calls = sent["messages"][2]["tool_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    for (index, raw) in raw.iter().enumerate() {
        assert_eq!(calls[index]["id"], format!("call-{index}"));
        assert_eq!(calls[index]["function"]["name"], "read");
        assert_eq!(calls[index]["function"]["arguments"], *raw);
    }
    let error: Value =
        serde_json::from_str(sent["messages"][3]["content"].as_str().unwrap()).unwrap();
    assert_eq!(error, json!({"is_error":true,"content":"missing"}));
    assert_eq!(sent["messages"][4]["content"], "missing");
}

#[tokio::test]
async fn unrepresentable_native_history_is_unchanged_and_rejected_before_http() {
    let mut cases = vec![
        message(json!({"type":"input_image","image_url":"fixture://image"})),
        message(json!({"type":"input_audio","audio_url":"fixture://audio"})),
        message(json!({"type":"refusal","refusal":"declined"})),
        captured(json!({"type":"reasoning","id":"reasoning","summary":[],
            "encrypted_content":"synthetic-opaque"})),
    ];
    let mut unknown = message(json!({"type":"output_text","text":"tail"}));
    unknown.native.as_mut().unwrap().data["item"]["future_semantics"] = json!("retained");
    cases.push(unknown);
    let mut annotation = message(json!({"type":"output_text","text":"tail",
        "annotations":[{"type":"citation","url":"fixture://source"}]}));
    cases.push(annotation.clone());
    annotation
        .native
        .as_mut()
        .unwrap()
        .extra
        .insert("future_semantics".into(), json!(true));
    cases.push(annotation);
    cases.push(captured(
        json!({"type":"function_call","id":"tool","call_id":"call",
        "name":"read","namespace":"foreign","arguments":" { invalid JSON\n"}),
    ));

    for item in cases {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let transport = transport(format!(
            "http://{}/chat/completions",
            listener.local_addr().unwrap()
        ));
        let mut input = request();
        input
            .messages
            .push(ModelMessage::CanonicalOutput(Box::new(item.clone())));
        if let Some(ModelOutputItem::ToolCall(call)) = &item.normalized {
            input.messages.push(ModelMessage::ToolResult {
                call_id: call.call_id.clone(),
                content: "not executed".into(),
                is_error: true,
            });
        }
        let before = input.messages.clone();
        assert_eq!(
            transport.encode(&input),
            Err(ModelError::UnsupportedCapability)
        );
        assert!(input.messages == before);
        tokio::select! {
            biased;
            result = transport.start(input) => {
                assert!(matches!(result, Err(ModelError::UnsupportedCapability)));
            }
            _ = listener.accept() => panic!("native compatibility must reject before HTTP"),
            _ = tokio::time::sleep(Duration::from_secs(1)) => panic!("bounded compatibility check"),
        }
        assert!(
            timeout(Duration::from_millis(25), listener.accept())
                .await
                .is_err()
        );
        assert!(
            matches!(before.get(1), Some(ModelMessage::CanonicalOutput(saved)) if **saved == item)
        );
    }
}
