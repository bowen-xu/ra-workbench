use super::*;

#[test]
fn default_reasoning_path_preserves_interleaved_i64_parts_ignores_duplicate_done_and_keeps_final_truth()
 {
    let mut decoder = decoder();
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 1024,
        tool_calls: 2,
    })
    .unwrap();
    for event in start(&mut decoder) {
        assembler.push(event).unwrap();
    }
    assert!(decoder.decode(&json!({"type":"response.reasoning_summary_text.delta","summary_index":i64::MAX,"delta":"orphan"}).to_string()).unwrap().is_empty());
    let added = json!({"type":"response.output_item.added","item":{"type":"reasoning","id":"rs","summary":[]}});
    let mut events = decoder.decode(&added.to_string()).unwrap();
    for (index, text) in [(i64::MIN, "A1你"), (i64::MAX, "B🦀"), (i64::MIN, "A2")] {
        events.extend(
            decoder
                .decode(
                    &json!({"type":"response.reasoning_summary_part.added","summary_index":index})
                        .to_string(),
                )
                .unwrap(),
        );
        events.extend(decoder.decode(&json!({"type":"response.reasoning_summary_text.delta","summary_index":index,"delta":text}).to_string()).unwrap());
    }
    assert!(events.iter().any(|event|matches!(event,ModelStreamEvent::ReasoningSummaryDelta{summary_index,..}if summary_index.as_str()==i64::MAX.to_string())));
    assert!(decoder.decode(&json!({"type":"response.reasoning_summary_text.done","item_id":"rs","summary_index":0,"text":"duplicate must not append"}).to_string()).unwrap().is_empty());
    let original = json!({"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"corrected你"},{"type":"summary_text","text":"🙂"}],"encrypted_content":"opaque-fixture"});
    events.extend(decoder.decode(&done(original.clone())).unwrap());
    assert!(decoder.decode(&json!({"type":"response.reasoning_summary_text.delta","summary_index":0,"delta":"late"}).to_string()).unwrap().is_empty());
    events.extend(decoder.decode(&completed(vec![])).unwrap());
    for event in events {
        assembler.push(event).unwrap();
    }
    let item = assembler.completed_items().into_iter().next().unwrap();
    assert_eq!(
        crate::agent::model::responses_native::render(&item).unwrap(),
        original
    );
    let response = assembler.finish().unwrap();
    assert_eq!(response.text, "");
    assert!(
        matches!(&response.output[..],[ModelOutputItem::ReasoningSummary(summary)]if summary.text=="corrected你🙂"&&summary.parts.len()==2)
    );
}

#[test]
fn duplicate_empty_progress_does_not_grow_preidentity_buffers_but_unique_parts_are_bounded() {
    let make = || {
        ResponsesDecoder::new(
            crate::agent::model::responses_native::NativeScope::fixture(),
            1024,
            2,
        )
        .unwrap()
    };
    let added=json!({"type":"response.output_item.added","item":{"type":"reasoning","id":"rs","summary":[]}}).to_string();
    let mut decoder = make();
    assert!(decoder.decode(&added).unwrap().is_empty());
    for _ in 0..5000 {
        for event in [
            json!({"type":"response.reasoning_summary_part.added","summary_index":i64::MAX}),
            json!({"type":"response.reasoning_summary_text.delta","summary_index":i64::MAX,"delta":""}),
        ] {
            assert!(decoder.decode(&event.to_string()).unwrap().is_empty());
        }
    }
    decoder
        .decode(&done(json!({"type":"reasoning","id":"rs","summary":[]})))
        .unwrap();
    let events = decoder.decode(&completed(vec![])).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ModelStreamEvent::ReasoningSummaryPartAdded { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ModelStreamEvent::ReasoningSummaryDelta { .. }))
            .count(),
        0
    );
    let mut bounded = make();
    bounded.decode(&added).unwrap();
    let error = (0..1024i64).find_map(|index| {
        bounded
            .decode(
                &json!({"type":"response.reasoning_summary_part.added","summary_index":index})
                    .to_string(),
            )
            .err()
    });
    assert_eq!(error, Some(ModelError::BudgetExceeded));
}

#[test]
fn sparse_reasoning_done_before_any_response_identity_is_released_after_start_without_invented_metadata()
 {
    let scope = crate::agent::model::responses_native::NativeScope::fixture();
    let mut decoder = ResponsesDecoder::new(scope.clone(), 4096, 2).unwrap();
    let original = json!({"type":"reasoning","id":"rs-sparse","summary":[{"type":"summary_text","text":"你🙂"}],"encrypted_content":"opaque-fixture"});
    assert!(decoder.decode(&done(original.clone())).unwrap().is_empty());
    let events = decoder
        .decode(
            &json!({"type":"response.completed","response":{"id":"only-terminal-id"}}).to_string(),
        )
        .unwrap();
    assert!(
        matches!(events.first(),Some(ModelStreamEvent::StartedWithoutModel{response_id})if response_id=="only-terminal-id")
    );
    let item = events
        .iter()
        .find_map(|event| match event {
            ModelStreamEvent::OutputItemDone(item) => Some(item),
            _ => None,
        })
        .unwrap();
    assert!(item.native.as_ref().unwrap().response_id.is_none());
    assert!(item.native.as_ref().unwrap().actual_model.is_none());
    assert_eq!(
        crate::agent::model::responses_native::render(item).unwrap(),
        original
    );
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 4096,
        argument_bytes: 1024,
        tool_calls: 2,
    })
    .unwrap()
    .with_requested_model(scope.model);
    for event in events {
        assembler.push(event).unwrap();
    }
    let response = assembler.finish().unwrap();
    assert!(!response.actual_model_observed && response.text.is_empty());
    assert!(
        matches!(&response.output[..],[ModelOutputItem::ReasoningSummary(summary)]if summary.text=="你🙂")
    );
}
