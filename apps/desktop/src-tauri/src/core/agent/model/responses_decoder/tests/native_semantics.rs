use super::*;
use research_agent_core::model::ModelOutputItem;

fn assemble(
    items: Vec<Value>,
    end_turn: Option<Value>,
) -> Result<research_agent_core::model::ModelResponse, ModelError> {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 4096,
        tool_calls: 2,
    })?;
    for event in start(&mut decoder) {
        assembler.push(event)?;
    }
    for item in &items {
        for event in decoder.decode(&done(item.clone()))? {
            assembler.push(event)?;
        }
    }
    let mut terminal: Value = serde_json::from_str(&completed(items)).unwrap();
    if let Some(value) = end_turn {
        terminal["response"]["end_turn"] = value;
    }
    for event in decoder.decode(&terminal.to_string())? {
        assembler.push(event)?;
    }
    assembler.finish()
}

#[test]
fn optional_codex_end_turn_reaches_shared_response_without_guessing() {
    for value in [
        None,
        Some(Value::Null),
        Some(json!(true)),
        Some(json!(false)),
    ] {
        let response = assemble(vec![message()], value.clone()).unwrap();
        assert_eq!(response.end_turn, value.and_then(|value| value.as_bool()));
    }
    for invalid in [json!("false"), json!(0), json!({})] {
        assert!(matches!(
            assemble(vec![message()], Some(invalid)),
            Err(ModelError::InvalidStream)
        ));
    }
}

#[test]
fn independent_message_phases_and_text_tool_order_survive_one_response() {
    let mut first = message();
    first["phase"] = json!("commentary");
    let mut last = message();
    last["id"] = json!("m2");
    last["phase"] = json!("final_answer");
    last["content"][0]["text"] = json!("Finished");
    let response = assemble(vec![first, call(), last], Some(json!(true))).unwrap();
    assert!(matches!(&response.output[..], [
        ModelOutputItem::Message(first), ModelOutputItem::ToolCall(call), ModelOutputItem::Message(last),
    ] if first.item_id == "m1" && first.text == "Hello" && first.phase == Some(AssistantMessagePhase::Commentary)
        && call.call_id == "call1" && last.item_id == "m2" && last.text == "Finished"
        && last.phase == Some(AssistantMessagePhase::FinalAnswer)));
    let unknown = assemble(vec![message()], None).unwrap();
    assert!(
        matches!(&unknown.output[..], [ModelOutputItem::Message(message)] if message.phase.is_none())
    );
}

#[test]
fn added_phase_and_late_done_phase_are_preserved_without_duplicate_text() {
    let mut decoder = decoder();
    start(&mut decoder);
    let mut item = message();
    item["phase"] = json!("commentary");
    let mut added = item.clone();
    added["content"] = json!([]);
    let events = decoder
        .decode(&json!({"type":"response.output_item.added", "item":added}).to_string())
        .unwrap();
    assert!(matches!(
        events.as_slice(),
        [ModelStreamEvent::AssistantMessageStarted {
            phase: Some(AssistantMessagePhase::Commentary),
            ..
        }]
    ));
    let events = decoder
        .decode(
            &json!({"type":"response.output_text.delta", "item_id":"m1", "delta":"Hello"})
                .to_string(),
        )
        .unwrap();
    assert!(
        matches!(events.as_slice(), [ModelStreamEvent::AssistantTextDelta { text, .. }] if text == "Hello")
    );
    assert!(matches!(
        decoder.decode(&done(item)).unwrap().as_slice(),
        [event] if completed_message(event).is_some_and(|message|message.phase==Some(AssistantMessagePhase::Commentary))
    ));
}

#[test]
fn unknown_native_phase_is_rejected() {
    for phase in [json!("reasoning"), json!("finalAnswer"), json!(true)] {
        let mut item = message();
        item["phase"] = phase;
        assert!(matches!(
            assemble(vec![item], None),
            Err(ModelError::InvalidStream)
        ));
    }
}

#[test]
fn authoritative_done_corrects_streamed_text_and_known_phase_in_one_item() {
    // Fixed Codex turn.rs::OutputItemDone finalizes the completed snapshot;
    // streamed assistant text and phase are draft presentation, not a CAS.
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
    let mut item = message();
    item["phase"] = json!("commentary");
    item["content"] = json!([]);
    for event in decoder
        .decode(&json!({"type":"response.output_item.added", "item":item}).to_string())
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    for event in decoder
        .decode(
            &json!({"type":"response.output_text.delta", "item_id":"m1", "delta":"draft"})
                .to_string(),
        )
        .unwrap()
    {
        assembler.push(event).unwrap();
    }
    let mut corrected = message();
    corrected["phase"] = json!("final_answer");
    corrected["content"][0]["text"] = json!("corrected");
    let events = decoder.decode(&done(corrected.clone())).unwrap();
    assert!(matches!(events.as_slice(), [event]
        if completed_message(event).is_some_and(|message|
            message.item_id=="m1" && message.text=="corrected"
            && message.phase==Some(AssistantMessagePhase::FinalAnswer))));
    for event in events {
        assembler.push(event).unwrap();
    }
    for event in decoder.decode(&completed(vec![corrected.clone()])).unwrap() {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(
        matches!(response.output.as_slice(), [ModelOutputItem::Message(message)]
        if message.item_id=="m1" && message.text=="corrected"
        && message.phase==Some(AssistantMessagePhase::FinalAnswer))
    );
    assert_eq!(decoder.take_completed_output().unwrap(), vec![corrected]);
}

#[test]
fn canonical_correction_keeps_item_identity_and_closed_item_boundaries() {
    fn streamed() -> ResponsesDecoder {
        let mut decoder = decoder();
        start(&mut decoder);
        let mut added = message();
        added["content"] = json!([]);
        decoder
            .decode(&json!({"type":"response.output_item.added", "item":added}).to_string())
            .unwrap();
        decoder
            .decode(
                &json!({"type":"response.output_text.delta", "item_id":"m1", "delta":"draft"})
                    .to_string(),
            )
            .unwrap();
        decoder
    }
    for (key, invalid) in [("id", json!("other")), ("role", json!("user"))] {
        let mut decoder = streamed();
        let mut item = message();
        item[key] = invalid;
        assert!(matches!(
            decoder.decode(&done(item)),
            Err(ModelError::InvalidStream)
        ));
    }
    for late in [
        done(message()),
        json!({"type":"response.output_text.delta", "item_id":"m1", "delta":"late"}).to_string(),
    ] {
        let mut decoder = streamed();
        assert!(
            matches!(decoder.decode(&done(message())).unwrap().as_slice(), [event]
            if completed_message(event).is_some_and(|message|message.text=="Hello"))
        );
        assert!(matches!(
            decoder.decode(&late),
            Err(ModelError::InvalidStream)
        ));
        assert!(decoder.decode(&completed(vec![message()])).is_err());
    }
}

#[test]
fn added_only_phase_is_retained_in_native_replay_after_terminal_validation() {
    for done_phase in [None, Some(Value::Null)] {
        let mut decoder = decoder();
        start(&mut decoder);
        let mut added = message();
        added["phase"] = json!("commentary");
        added["content"] = json!([]);
        decoder
            .decode(&json!({"type":"response.output_item.added", "item":added}).to_string())
            .unwrap();
        let mut item = message();
        if let Some(phase) = done_phase {
            item["phase"] = phase;
        }
        decoder.decode(&done(item.clone())).unwrap();
        decoder.decode(&completed(vec![item])).unwrap();
        let output = decoder.take_completed_output().unwrap();
        assert_eq!(output[0]["phase"], "commentary");
    }
}
