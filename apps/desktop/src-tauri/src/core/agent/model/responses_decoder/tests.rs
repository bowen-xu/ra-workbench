use super::*;
use research_agent_core::model::{
    AssistantMessage, ModelOutputItem, ResponseAssembler, StreamLimits, ToolCallIntent,
};
use serde_json::json;

fn start(decoder: &mut ResponsesDecoder) -> Vec<ModelStreamEvent> {
    decoder
        .decode(
            &json!({"type":"response.created","response":{"id":"r1","model":"fixture","headers":{"OpenAI-Model":"fixture"}}})
                .to_string(),
        )
        .unwrap()
}
fn done(item: Value) -> String {
    json!({"type":"response.output_item.done","item":item}).to_string()
}
fn completed(output: Vec<Value>) -> String {
    json!({"type":"response.completed","response":{"id":"r1","model":"fixture","status":"completed",
        "output":output,"usage":{"input_tokens":20,"output_tokens":10}}})
    .to_string()
}
fn message() -> Value {
    json!({"id":"m1","type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]})
}
fn call() -> Value {
    json!({"id":"f1","type":"function_call","call_id":"call1","name":"read","arguments":"{}"})
}
fn decoder() -> ResponsesDecoder {
    ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope::fixture(),
        4096,
        2,
    )
    .unwrap()
}
fn completed_message(event: &ModelStreamEvent) -> Option<&AssistantMessage> {
    match event {
        ModelStreamEvent::OutputItemDone(item) => match &item.normalized {
            Some(ModelOutputItem::Message(message)) => Some(message),
            _ => None,
        },
        _ => None,
    }
}
fn completed_call(event: &ModelStreamEvent) -> Option<&ToolCallIntent> {
    match event {
        ModelStreamEvent::OutputItemDone(item) => match &item.normalized {
            Some(ModelOutputItem::ToolCall(call)) => Some(call),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn additive_progress_events_do_not_abort_text_or_authorize_tools() {
    let mut decoder = decoder();
    start(&mut decoder);
    for kind in [
        "response.model_verification",
        "response.reasoning_summary.started",
        "response.custom_tool_call_input.delta",
    ] {
        assert!(decoder.decode(&json!({"type":kind,"delta":"not assistant text","name":"execute","arguments":"{}"}).to_string()).unwrap().is_empty());
    }
    decoder
        .decode(
            &json!({"type":"response.output_text.delta","item_id":"m1","delta":"Hello"})
                .to_string(),
        )
        .unwrap();
    // Late metadata used to discard otherwise complete generated text.
    assert!(decoder.decode(&json!({"type":"response.output_text.annotation.added","item_id":"m1","annotation":{"type":"future_annotation"}}).to_string()).unwrap().is_empty());
    assert!(
        matches!(decoder.decode(&done(message())).unwrap().as_slice(),
        [event] if completed_message(event).is_some_and(|message|message.item_id=="m1"))
    );
    assert!(
        decoder
            .decode(&completed(vec![message()]))
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None
                }
            ))
    );
    assert_eq!(decoder.take_completed_output().unwrap(), vec![message()]);
}

#[test]
fn ignored_progress_cannot_replace_terminal_validation_or_hide_provider_errors() {
    let mut decoder = decoder();
    start(&mut decoder);
    decoder
        .decode(
            &json!({"type":"response.future_progress","response":{"status":"completed"}})
                .to_string(),
        )
        .unwrap();
    assert_eq!(
        decoder.take_completed_output(),
        Err(ModelError::IncompleteStream)
    );
    assert!(matches!(
        decoder.decode(
            &json!({"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}})
                .to_string()
        ),
        Err(ModelError::RateLimited)
    ));
}

#[test]
fn sparse_terminal_preserves_done_order_without_requiring_output_indices() {
    for indices in [vec![1, 0], vec![0, 0], vec![0, 2]] {
        let mut decoder = decoder();
        start(&mut decoder);
        for (item, index) in [message(), call()].into_iter().zip(indices) {
            let events = decoder
                .decode(
                    &json!({"type":"response.output_item.done", "output_index":index, "item":item})
                        .to_string(),
                )
                .unwrap();
            // A done item is observable independently of response completion.
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, ModelStreamEvent::Completed { .. }))
            );
        }
        let result = decoder.decode(&completed(vec![])).unwrap();
        assert!(result.iter().any(|event| matches!(
            event,
            ModelStreamEvent::Completed {
                reason: FinishReason::ToolCalls,
                end_turn: None
            }
        )));
        assert_eq!(
            decoder.take_completed_output().unwrap(),
            vec![message(), call()]
        );
    }
    let mut unfinished = decoder();
    start(&mut unfinished);
    unfinished
        .decode(
            &json!({"type":"response.output_item.added", "item":{
                "id":"unfinished", "type":"message", "role":"assistant", "content":[]
            }})
            .to_string(),
        )
        .unwrap();
    assert!(unfinished.decode(&completed(vec![])).is_err());
}

#[test]
fn streams_text_once_and_retains_reasoning_for_native_replay() {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })
    .unwrap();
    for event in start(&mut decoder) {
        assembler.push(event).unwrap();
    }
    let reasoning = json!({"id":"reason1","type":"reasoning","summary":[],"encrypted_content":"opaque-fixture"});
    for payload in [
        done(reasoning.clone()),
        json!({"type":"response.output_text.delta","item_id":"m1","delta":"Hel"}).to_string(),
        done(message()),
        done(call()),
        completed(vec![reasoning.clone(), message(), call()]),
    ] {
        for event in decoder.decode(&payload).unwrap() {
            assembler.push(event).unwrap();
        }
    }
    let response = assembler.finish().unwrap();
    assert_eq!(response.text, "Hello");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].call_id, "call1");
    assert_eq!(response.usage.input_tokens, Some(20));
    assert_eq!(
        decoder.take_completed_output().unwrap(),
        vec![reasoning, message(), call()]
    );
}

#[test]
fn incomplete_response_does_not_claim_a_successful_terminal() {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })
    .unwrap();
    for event in start(&mut decoder)
        .into_iter()
        .chain(decoder.decode(&done(call())).unwrap())
    {
        assembler.push(event).unwrap();
    }
    assert!(matches!(
        assembler.finish(),
        Err(ModelError::IncompleteStream)
    ));
    assert!(decoder.take_completed_output().is_err());
}

#[test]
fn mismatch_duplicate_and_failure_poison_the_decoder() {
    for payload in [json!({"type":"response.completed","response":{"id":"wrong"}}).to_string(), json!({"type":"response.failed"}).to_string(),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}).to_string()] {
        let mut decoder=decoder(); start(&mut decoder);
        assert!(decoder.decode(&payload).is_err());
        assert!(decoder.take_completed_output().is_err());
        assert!(decoder.decode(&completed(vec![])).is_err());
    }
    let mut decoder = decoder();
    start(&mut decoder);
    decoder.decode(&done(call())).unwrap();
    assert!(decoder.decode(&done(call())).is_err());
    assert!(decoder.take_completed_output().is_err());
}

#[test]
fn enforces_retained_state_budget_and_accepts_unknown_tool_names() {
    let mut decoder = ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope::fixture(),
        200,
        2,
    )
    .unwrap();
    start(&mut decoder);
    for index in 0..10 {
        let item =
            json!({"id":format!("reason{index}"),"type":"reasoning","encrypted_content":"bounded"});
        if let Err(error) = decoder.decode(&done(item)) {
            assert_eq!(error, ModelError::BudgetExceeded);
            return;
        }
    }
    panic!("retained output exceeded configured budget");
}

#[test]
fn done_only_text_is_visible_and_canonical_correction_is_not_appended() {
    let mut first = decoder();
    start(&mut first);
    assert!(
        matches!(first.decode(&done(message())).unwrap().as_slice(),[
            ModelStreamEvent::AssistantMessageStarted { item_id, .. },
            ModelStreamEvent::AssistantTextDelta { text, .. },
            ModelStreamEvent::OutputItemDone(_),
        ] if item_id == "m1" && text=="Hello")
    );
    let mut second = decoder();
    start(&mut second);
    second
        .decode(
            &json!({"type":"response.output_text.delta","item_id":"m1","delta":"Different"})
                .to_string(),
        )
        .unwrap();
    let events = second.decode(&done(message())).unwrap();
    assert!(matches!(events.as_slice(), [event]
        if completed_message(event).is_some_and(|message| message.item_id=="m1" && message.text=="Hello")));
    second.decode(&completed(vec![message()])).unwrap();
    assert_eq!(second.take_completed_output().unwrap(), vec![message()]);
    let mut third = decoder();
    start(&mut third);
    assert!(matches!(
        third.decode(&done(json!({"id":"unknown","type":"computer_call"}))),
        Err(ModelError::UnsupportedResponseType)
    ));
}

#[test]
fn terminal_output_cannot_reorder_already_admitted_parallel_calls() {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })
    .unwrap();
    for event in start(&mut decoder) {
        assembler.push(event).unwrap();
    }
    let first = call();
    let mut second = call();
    second["id"] = json!("f2");
    second["call_id"] = json!("call2");
    for payload in [
        done(second.clone()),
        done(first.clone()),
        completed(vec![first, second]),
    ] {
        for event in decoder.decode(&payload).unwrap() {
            assembler.push(event).unwrap();
        }
    }
    let response = assembler.finish().unwrap();
    let ids = response
        .tool_calls
        .iter()
        .map(|call| call.call_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["call2", "call1"]);
}

#[path = "tests/native_semantics.rs"]
mod native_semantics;
#[path = "tests/tool_done.rs"]
mod tool_done;

#[path = "tests/codex_compatibility.rs"]
mod codex_compatibility;
#[path = "tests/reasoning.rs"]
mod reasoning;

#[test]
fn native_done_is_owned_before_a_later_failure_and_over_budget_done_is_never_emitted() {
    let mut decoder = decoder();
    start(&mut decoder);
    let native =
        json!({"type":"reasoning","id":"rs-owned","encrypted_content":"owned-opaque","summary":[]});
    let events = decoder.decode(&done(native.clone())).unwrap();
    let [ModelStreamEvent::OutputItemDone(item)] = events.as_slice() else {
        panic!("reasoning done must be independently accepted")
    };
    assert!(
        matches!(&item.normalized,Some(ModelOutputItem::ReasoningSummary(summary))if summary.text.is_empty()&&summary.parts.is_empty())
    );
    assert!(decoder.decode(&json!({"type":"response.failed","response":{"id":"r1","error":{"code":"server_error"}}}).to_string()).is_err());
    assert_eq!(
        crate::agent::model::responses_native::render(item).unwrap(),
        native
    );
    let mut bounded = ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope::fixture(),
        200,
        2,
    )
    .unwrap();
    start(&mut bounded);
    // Wire bytes fit; the complete native envelope exceeds the admission cap.
    assert!(done(native.clone()).len() < 200);
    assert!(matches!(
        bounded.decode(&done(native)),
        Err(ModelError::BudgetExceeded)
    ));
}
