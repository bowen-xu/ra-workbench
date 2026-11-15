use super::*;
use research_agent_core::model::{ResponseAssembler, StreamLimits};
use serde_json::{Value, json};

fn decoder() -> ChatDecoder {
    ChatDecoder::new("fixture".into(), 2, 4096).unwrap()
}
fn chunk(delta: Value, reason: Option<&str>) -> String {
    json!({"id":"response-1", "model":"actual-model", "object":"chat.completion.chunk",
        "choices":[{"index":0,"delta":delta,"finish_reason":reason}]})
    .to_string()
}
fn tool(index: u32, id: &str, args: &str) -> Value {
    json!({"index":index,"id":id,"type":"function",
        "function":{"name":"read","arguments":args}})
}
fn assemble(payloads: &[String]) -> Result<research_agent_core::model::ModelResponse, ModelError> {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })
    .unwrap();
    for data in payloads {
        for event in decoder.decode(data)? {
            assembler.push(event)?;
        }
    }
    assembler.finish()
}

#[test]
fn interleaved_tool_chunks_and_trailing_usage_reach_core_without_losing_identity() {
    let response = assemble(&[
        chunk(json!({"role":"assistant","content":"检查文稿"}), None),
        chunk(
            json!({"tool_calls":[tool(0,"call-a","{\"path\":"),tool(1,"call-b","{}")]}),
            None,
        ),
        chunk(
            json!({"tool_calls":[{"index":0,"id":null,"type":null,
            "function":{"name":null,"arguments":"\"a.tex\"}"}}]}),
            Some("tool_calls"),
        ),
        json!({"id":"response-1","model":"actual-model","object":"chat.completion.chunk",
            "choices":[],"usage":{"prompt_tokens":35,"completion_tokens":12}})
        .to_string(),
        "[DONE]".into(),
    ])
    .unwrap();
    assert_eq!(response.text, "检查文稿");
    assert_eq!(response.actual_model.model_id, "actual-model");
    assert_eq!(response.tool_calls[0].call_id, "call-a");
    assert_eq!(
        response.tool_calls[0].raw_arguments_json,
        "{\"path\":\"a.tex\"}"
    );
    assert_eq!(response.tool_calls[1].call_id, "call-b");
    assert_eq!(response.usage.input_tokens, Some(35));
    assert_eq!(response.usage.output_tokens, Some(12));
}

#[test]
fn empty_continuation_id_preserves_the_established_tool_call() {
    let response = assemble(&[
        chunk(json!({"tool_calls":[tool(0,"call-a","{\"path\":")]}), None),
        chunk(
            json!({"tool_calls":[{"index":0,"id":"","type":"function",
                "function":{"arguments":"\"a.tex\"}"}}]}),
            Some("tool_calls"),
        ),
        "[DONE]".into(),
    ])
    .unwrap();
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].call_id, "call-a");
    assert_eq!(response.tool_calls[0].name, "read");
    assert_eq!(
        response.tool_calls[0].raw_arguments_json,
        "{\"path\":\"a.tex\"}"
    );
}

#[test]
fn empty_id_cannot_establish_a_tool_call_or_a_new_index() {
    let mut parser = decoder();
    assert!(matches!(
        parser.decode(&chunk(json!({"tool_calls":[tool(0,"","{}")]}), None)),
        Err(ModelError::InvalidStream)
    ));
    let mut parser = decoder();
    parser
        .decode(&chunk(json!({"tool_calls":[tool(0,"call-a","")]}), None))
        .unwrap();
    assert!(matches!(
        parser.decode(&chunk(json!({"tool_calls":[tool(1,"","{}")]}), None)),
        Err(ModelError::InvalidStream)
    ));
}

#[test]
fn missing_finish_or_done_cannot_release_tools() {
    let pending = chunk(json!({"tool_calls":[tool(0,"a","{}")]}), None);
    assert!(matches!(
        assemble(&[pending.clone(), "[DONE]".into()]),
        Err(ModelError::IncompleteStream)
    ));
    assert!(matches!(
        assemble(&[pending, chunk(json!({}), Some("tool_calls"))]),
        Err(ModelError::IncompleteStream)
    ));
}

#[test]
fn completed_calls_keep_invalid_json_and_non_object_strings_for_handler_validation() {
    for raw in ["", "{", "[1]", "null", "17", "\"text\""] {
        let response = assemble(&[
            chunk(
                json!({"tool_calls":[tool(0,"call",raw)]}),
                Some("tool_calls"),
            ),
            "[DONE]".into(),
        ])
        .unwrap();
        assert_eq!(response.tool_calls[0].raw_arguments_json, raw);
    }
}

#[test]
fn truncation_and_filtering_do_not_parse_incomplete_tool_arguments() {
    for (reason, error) in [
        ("length", ModelError::Truncated),
        ("content_filter", ModelError::Filtered),
    ] {
        let result = assemble(&[
            chunk(
                json!({"tool_calls":[tool(0,"a","{\"partial")]}),
                Some(reason),
            ),
            "[DONE]".into(),
        ]);
        assert!(matches!(result, Err(actual) if actual == error));
    }
}

#[test]
fn invalid_chunks_poison_decoder_and_cannot_be_repaired_by_done() {
    let invalid = [
        json!({"index":1,"delta":{},"finish_reason":"stop"}),
        json!({"index":0,"delta":{"role":"user"},"finish_reason":"stop"}),
        json!({"index":0,"delta":{"audio":{}},"finish_reason":"stop"}),
        json!({"index":0,"delta":{"function_call":{}},"finish_reason":"stop"}),
        json!({"index":0,"delta":{},"finish_reason":"function_call"}),
        json!({"index":0,"delta":{},"finish_reason":"tool_calls"}),
    ];
    for choice in invalid {
        let mut decoder = decoder();
        let payload =
            json!({"id":"r","model":"m","object":"chat.completion.chunk","choices":[choice]});
        assert!(decoder.decode(&payload.to_string()).is_err());
        assert!(matches!(
            decoder.decode("[DONE]"),
            Err(ModelError::InvalidStream)
        ));
    }
}

#[test]
fn response_and_tool_identity_cannot_change_midstream() {
    for delta in [
        json!({"tool_calls":[tool(0,"different","{}")]}),
        json!({"tool_calls":[tool(1,"a","{}")]}),
        json!({"tool_calls":[{"index":0,"function":{"name":"write","arguments":"{}"}}]}),
    ] {
        let mut decoder = decoder();
        decoder
            .decode(&chunk(json!({"tool_calls":[tool(0,"a","")]}), None))
            .unwrap();
        assert!(matches!(
            decoder.decode(&chunk(delta, None)),
            Err(ModelError::InvalidStream)
        ));
    }
    for field in ["model", "id"] {
        let mut decoder = decoder();
        decoder.decode(&chunk(json!({}), None)).unwrap();
        let mut payload: Value = serde_json::from_str(&chunk(json!({}), None)).unwrap();
        payload[field] = json!("different");
        assert!(matches!(
            decoder.decode(&payload.to_string()),
            Err(ModelError::InvalidStream)
        ));
    }
}

#[test]
fn budgets_bound_wire_payload_and_tool_index_storage() {
    let mut decoder = decoder();
    assert!(matches!(
        decoder.decode(&"x".repeat(4097)),
        Err(ModelError::BudgetExceeded)
    ));
    let mut decoder = ChatDecoder::new("fixture".into(), 1, 4096).unwrap();
    decoder
        .decode(&chunk(
            json!({"tool_calls":[tool(u32::MAX,"a","{}")]}),
            None,
        ))
        .unwrap();
    assert!(matches!(
        decoder.decode(&chunk(json!({"tool_calls":[tool(0,"b","{}")]}), None)),
        Err(ModelError::BudgetExceeded)
    ));
}

#[test]
fn refusal_is_visible_and_terminal_choice_rejects_late_content() {
    let response = assemble(&[
        chunk(json!({"refusal":"Cannot assist"}), Some("stop")),
        "[DONE]".into(),
    ])
    .unwrap();
    assert_eq!(response.text, "Cannot assist");
    let mut decoder = decoder();
    decoder.decode(&chunk(json!({}), Some("stop"))).unwrap();
    assert!(matches!(
        decoder.decode(&chunk(json!({"content":"late"}), None)),
        Err(ModelError::InvalidStream)
    ));
}
