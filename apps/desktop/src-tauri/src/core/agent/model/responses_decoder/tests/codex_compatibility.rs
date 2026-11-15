//! Regression contracts from fixed Codex e363b08 sse/responses.rs fixtures.
//! Copyright 2025 OpenAI. Adapted under Apache-2.0.
use super::*;
use research_agent_core::model::ModelOutputItem;

fn assembler() -> ResponseAssembler {
    ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })
    .unwrap()
    .with_requested_model(ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "configured-fallback".into(),
        revision: None,
    })
}

#[test]
fn fixed_codex_parses_items_and_completed_without_created_model_or_item_ids() {
    let mut decoder = decoder();
    let mut assembler = assembler();
    // Equivalent to Codex parses_items_and_completed, including missing IDs.
    for (text, phase) in [("Hello", Some("commentary")), ("World", None)] {
        let mut item = json!({"type":"message", "role":"assistant",
            "content":[{"type":"output_text", "text":text}]});
        if let Some(phase) = phase {
            item["phase"] = json!(phase);
        }
        assert!(decoder.decode(&done(item)).unwrap().is_empty());
    }
    assert!(decoder.take_completed_output().is_err());
    let events = decoder
        .decode(
            &json!({"type":"response.completed",
        "response":{"id":"resp1"}})
            .to_string(),
        )
        .unwrap();
    assert!(
        matches!(&events[0], ModelStreamEvent::StartedWithoutModel { response_id } if response_id == "resp1")
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert_eq!(response.response_id, "resp1");
    assert_eq!(response.actual_model.model_id, "configured-fallback");
    assert!(!response.actual_model_observed);
    assert!(
        matches!(&response.output[..], [ModelOutputItem::Message(first), ModelOutputItem::Message(last)]
        if first.item_id != last.item_id && first.text == "Hello" && last.text == "World"
        && first.phase == Some(AssistantMessagePhase::Commentary) && last.phase.is_none())
    );
    let retained = decoder.take_completed_output().unwrap();
    for (retained, normalized) in retained.iter().zip(&response.output) {
        let ModelOutputItem::Message(normalized) = normalized else {
            panic!("expected message")
        };
        assert_eq!(retained["id"], normalized.item_id);
    }
}

#[test]
fn completed_ignores_additive_status_like_fixed_codex() {
    for status in [
        json!("in_progress"),
        json!("future-status"),
        json!({"vendor":"opaque"}),
    ] {
        let mut decoder = decoder();
        let mut assembled = assembler();
        for event in decoder.decode(&done(message())).unwrap() {
            assembled.push(event).unwrap();
        }
        let terminal = json!({"type":"response.completed","response":{"id":"r1","status":status}});
        for event in decoder.decode(&terminal.to_string()).unwrap() {
            assembled.push(event).unwrap();
        }
        let response = assembled.finish().unwrap();
        assert_eq!(response.response_id, "r1");
        assert!(
            matches!(&response.output[..], [ModelOutputItem::Message(message)] if message.text == "Hello")
        );
    }
}

#[test]
fn missing_done_id_uses_active_item_and_preserves_its_phase() {
    let mut decoder = decoder();
    decoder
        .decode(&json!({"type":"response.created","response":{"id":"r1"}}).to_string())
        .unwrap();
    let mut added = message();
    added["phase"] = json!("commentary");
    added["content"] = json!([]);
    decoder
        .decode(&json!({"type":"response.output_item.added","item":added}).to_string())
        .unwrap();
    let delta = decoder
        .decode(&json!({"type":"response.output_text.delta","delta":"Hel"}).to_string())
        .unwrap();
    assert!(
        matches!(&delta[..], [ModelStreamEvent::AssistantTextDelta {item_id,text}] if item_id == "m1" && text == "Hel")
    );
    let mut item = message();
    item.as_object_mut().unwrap().remove("id");
    let events = decoder.decode(&done(item)).unwrap();
    assert!(
        matches!(&events[..], [ModelStreamEvent::AssistantTextDelta {item_id,text},
        event]
        if item_id == "m1" && text == "lo" && completed_message(event).is_some_and(|message|message.phase==Some(AssistantMessagePhase::Commentary)))
    );
    decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap();
    let retained = decoder.take_completed_output().unwrap();
    assert_eq!(retained[0]["id"], "m1");
    assert_eq!(retained[0]["phase"], "commentary");
}

#[test]
fn sparse_created_streams_done_immediately_and_late_model_is_observed_once() {
    let mut decoder = decoder();
    let mut assembler = assembler();
    let events = decoder
        .decode(&json!({"type":"response.created","response":{"id":"r1"}}).to_string())
        .unwrap();
    assert!(matches!(
        &events[..],
        [ModelStreamEvent::StartedWithoutModel { .. }]
    ));
    for event in events {
        assembler.push(event).unwrap();
    }
    let events = decoder.decode(&done(message())).unwrap();
    assert!(
        events
            .iter()
            .any(|event| completed_message(event).is_some())
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    let progress = json!({"type":"response.in_progress","response":{"id":"r1",
        "headers":{"x-OPENAI-model":["served-model"]}}})
    .to_string();
    let events = decoder.decode(&progress).unwrap();
    assert!(
        matches!(&events[..], [ModelStreamEvent::ModelIdentityObserved(model)] if model.model_id == "served-model")
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    assert!(decoder.decode(&progress).unwrap().is_empty());
    for event in decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(response.actual_model_observed);
    assert_eq!(response.actual_model.model_id, "served-model");
}

#[test]
fn fixed_codex_created_response_headers_observe_model_without_body_model() {
    let mut decoder = decoder();
    let events = decoder
        .decode(
            &json!({"type":"response.created","response":{"id":"r1",
        "headers":{"OpenAI-Model":"served-model"}}})
            .to_string(),
        )
        .unwrap();
    assert!(
        matches!(&events[..], [ModelStreamEvent::Started {actual_model, ..}] if actual_model.model_id == "served-model")
    );
    decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap();
}

#[test]
fn fixed_codex_body_model_alias_is_not_an_observed_server_model() {
    // Fixed upstream process_sse_ignores_response_model_field_in_payload.
    let mut decoder = decoder();
    let mut assembler = assembler();
    let events = decoder
        .decode(
            &json!({"type":"response.created","response":{
                "id":"r1","model":"requested-alias"
            }})
            .to_string(),
        )
        .unwrap();
    assert!(matches!(
        &events[..],
        [ModelStreamEvent::StartedWithoutModel { .. }]
    ));
    for event in events {
        assembler.push(event).unwrap();
    }
    assert!(
        decoder
            .decode(
                &json!({"type":"response.in_progress","response":{
                    "id":"r1","model":"another-alias"
                }})
                .to_string()
            )
            .unwrap()
            .is_empty()
    );
    for event in decoder.decode(&done(message())).unwrap() {
        assembler.push(event).unwrap();
    }
    for event in decoder
        .decode(
            &json!({"type":"response.completed","response":{
                "id":"r1","model":"terminal-alias"
            }})
            .to_string(),
        )
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(!response.actual_model_observed);
    assert_eq!(response.actual_model.model_id, "configured-fallback");
}

#[test]
fn server_header_precedes_body_alias_and_later_header_observations_can_change() {
    let mut decoder = decoder();
    let mut assembler = assembler();
    let events = decoder.decode(&json!({"type":"response.created",
        "headers":{"OpenAI-Model":"top-level-fallback"},
        "response":{"id":"r1","model":"requested-alias","headers":{"OpenAI-Model":"served-v1"}}
    }).to_string()).unwrap();
    assert!(
        matches!(&events[..],[ModelStreamEvent::Started {actual_model,..}] if actual_model.model_id == "served-v1")
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    for event in decoder.decode(&done(message())).unwrap() {
        assembler.push(event).unwrap();
    }
    for (event, model) in [
        (
            json!({"type":"response.in_progress","response":{"id":"r1","model":"body-alias",
            "headers":{"x-openai-model":["served-v2"]}}}),
            "served-v2",
        ),
        (
            json!({"type":"response.metadata","headers":{"OpenAI-Model":"served-v3"}}),
            "served-v3",
        ),
    ] {
        let encoded = event.to_string();
        let events = decoder.decode(&encoded).unwrap();
        assert!(
            matches!(&events[..],[ModelStreamEvent::ModelIdentityObserved(observed)] if observed.model_id == model)
        );
        for event in events {
            assembler.push(event).unwrap();
        }
        assert!(decoder.decode(&encoded).unwrap().is_empty());
    }
    for event in decoder
        .decode(
            &json!({"type":"response.completed","response":{
                "id":"r1","model":"requested-alias"
            }})
            .to_string(),
        )
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(response.actual_model_observed);
    assert_eq!(response.actual_model.model_id, "served-v3");
}

#[test]
fn http_server_model_waits_for_real_response_id_and_accepts_later_sse_update() {
    let mut decoder = decoder()
        .with_server_model("http-served-v1".into())
        .unwrap();
    let mut assembler = assembler();
    assert!(
        decoder
            .decode(
                &json!({"type":"response.created","response":{
                    "model":"requested-alias"
                }})
                .to_string()
            )
            .unwrap()
            .is_empty()
    );
    assert!(decoder.decode(&done(message())).unwrap().is_empty());
    let events = decoder
        .decode(
            &json!({"type":"response.in_progress","response":{
                "id":"real-response-id","model":"body-alias"
            }})
            .to_string(),
        )
        .unwrap();
    assert!(
        matches!(&events[0],ModelStreamEvent::Started {response_id,actual_model}
        if response_id == "real-response-id" && actual_model.model_id == "http-served-v1")
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    let events = decoder
        .decode(
            &json!({"type":"response.metadata","headers":{
                "OpenAI-Model":"sse-served-v2"
            }})
            .to_string(),
        )
        .unwrap();
    assert!(
        matches!(&events[..],[ModelStreamEvent::ModelIdentityObserved(model)] if model.model_id == "sse-served-v2")
    );
    for event in events {
        assembler.push(event).unwrap();
    }
    for event in decoder
        .decode(
            &json!({"type":"response.completed","response":{
                "id":"real-response-id","model":"requested-alias"
            }})
            .to_string(),
        )
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(response.actual_model_observed);
    assert_eq!(response.actual_model.model_id, "sse-served-v2");
}

#[test]
fn complete_tool_item_uses_active_identity_before_sparse_terminal_arrives() {
    let mut decoder = decoder();
    decoder
        .decode(&json!({"type":"response.created","response":{"id":"r1"}}).to_string())
        .unwrap();
    let added = json!({"type":"function_call","id":"fc_active","call_id":"call1","name":"read","arguments":""});
    assert!(
        decoder
            .decode(&json!({"type":"response.output_item.added","item":added}).to_string())
            .unwrap()
            .is_empty()
    );
    let mut complete = call();
    complete.as_object_mut().unwrap().remove("id");
    let events = decoder.decode(&done(complete)).unwrap();
    assert!(
        matches!(&events[..], [event] if completed_call(event).is_some_and(|call|call.call_id=="call1" && call.raw_arguments_json=="{}"))
    );
    assert!(decoder.take_completed_output().is_err());
    decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap();
    assert_eq!(
        decoder.take_completed_output().unwrap()[0]["id"],
        "fc_active"
    );
}

#[test]
fn terminal_snapshot_cannot_replace_authoritative_done_items_or_native_phase() {
    let mut decoder = decoder();
    start(&mut decoder);
    let mut item = message();
    item["phase"] = json!("commentary");
    decoder.decode(&done(item.clone())).unwrap();
    let mut snapshot = item.clone();
    snapshot["annotations"] = json!([{"type":"future_metadata"}]);
    let events = decoder.decode(&completed(vec![snapshot, call()])).unwrap();
    assert!(!events.iter().any(|event| matches!(
        event,
        ModelStreamEvent::OutputItemDone(_) | ModelStreamEvent::OutputOrder(_)
    )));
    assert_eq!(decoder.take_completed_output().unwrap(), vec![item]);
}

#[test]
fn explicit_identity_conflicts_and_unattributable_done_are_rejected() {
    for event in [
        json!({"type":"response.in_progress","response":{"id":"other"}}),
        json!({"type":"response.completed","response":{"id":"r1"},"response_id":"other"}),
        json!({"type":"response.completed","response":{}}),
    ] {
        let mut decoder = decoder();
        start(&mut decoder);
        assert_eq!(
            decoder.decode(&event.to_string()).unwrap_err(),
            ModelError::InvalidStream
        );
    }
    for explicit_id in [None, Some("wrong")] {
        let mut decoder = decoder();
        start(&mut decoder);
        decoder
            .decode(&json!({"type":"response.output_item.added","item":message()}).to_string())
            .unwrap();
        let mut item = if explicit_id.is_some() {
            message()
        } else {
            call()
        };
        if let Some(id) = explicit_id {
            item["id"] = json!(id);
        } else {
            item.as_object_mut().unwrap().remove("id");
        }
        assert_eq!(
            decoder.decode(&done(item)).unwrap_err(),
            ModelError::InvalidStream
        );
    }
    let mut decoder = decoder();
    start(&mut decoder);
    decoder
        .decode(&json!({"type":"response.output_item.added","item":message()}).to_string())
        .unwrap();
    let mut changed_kind = call();
    changed_kind["id"] = json!("m1");
    assert_eq!(
        decoder.decode(&done(changed_kind)).unwrap_err(),
        ModelError::InvalidStream
    );
}

#[test]
fn explicit_parallel_done_keeps_its_own_pending_identity() {
    let mut decoder = decoder();
    start(&mut decoder);
    let first = call();
    let mut second = call();
    second["id"] = json!("f2");
    second["call_id"] = json!("call2");
    for item in [&first, &second] {
        decoder
            .decode(&json!({"type":"response.output_item.added","item":item}).to_string())
            .unwrap();
    }
    decoder.decode(&done(first.clone())).unwrap();
    decoder.decode(&done(second.clone())).unwrap();
    decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap();
    assert_eq!(
        decoder.take_completed_output().unwrap(),
        vec![first, second]
    );
}

#[test]
fn refusal_truncation_and_pre_identity_buffer_keep_existing_error_boundaries() {
    let mut decoder = decoder();
    start(&mut decoder);
    let refusal = json!({"id":"m1","type":"message","role":"assistant",
        "content":[{"type":"refusal","refusal":"Cannot comply"}]});
    let events = decoder.decode(&done(refusal)).unwrap();
    assert!(events.iter().any(|event| matches!(event,ModelStreamEvent::AssistantTextDelta {text,..} if text == "Cannot comply")));
    assert_eq!(
        decoder
            .decode(
                &json!({"type":"response.incomplete","response":{"incomplete_details":{
        "reason":"content_filter"}}})
                .to_string()
            )
            .unwrap_err(),
        ModelError::Filtered
    );
    let mut decoder = ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope::fixture(),
        250,
        2,
    )
    .unwrap();
    let mut exceeded = false;
    for index in 0..10 {
        let event = done(
            json!({"type":"reasoning","id":format!("r{index}"),"encrypted_content":"bounded"}),
        );
        match decoder.decode(&event) {
            Ok(events) => assert!(events.is_empty()),
            Err(error) => {
                assert_eq!(error, ModelError::BudgetExceeded);
                exceeded = true;
                break;
            }
        }
    }
    assert!(exceeded);
    assert!(decoder.take_completed_output().is_err());
}
