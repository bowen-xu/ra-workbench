use super::*;

#[test]
fn reasoning_binding_flag_cannot_be_ignored_by_message_or_function_templates() {
    for original in [
        json!({"type":"message","id":"msg","role":"assistant","content":[{"type":"output_text","text":"body"}]}),
        json!({"type":"function_call","id":"fc","call_id":"call","name":"read","arguments":"{}"}),
    ] {
        let mut item = capture(&original, &scope(), None, None).unwrap();
        item.native.as_mut().unwrap().data["summary_from_common"] = json!(true);
        assert_eq!(render(&item), Err(ModelError::InvalidStream));
    }
}

#[test]
fn public_parts_have_one_utf8_body_and_exact_reversible_native_bindings() {
    let original = json!({"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"第一","future":1},{"type":"summary_text","text":""},{"type":"summary_text","text":"🙂"}],"encrypted_content":"opaque-fixture","content":[{"type":"reasoning_text","text":"raw-private-fixture"}],"future":{"proof":true}});
    let item = capture(&original, &scope(), None, None).unwrap();
    let Some(ModelOutputItem::ReasoningSummary(summary)) = &item.normalized else {
        panic!("public summary")
    };
    assert_eq!(summary.text, "第一🙂");
    assert_eq!(
        summary
            .parts
            .iter()
            .map(|part| (part.index.as_str(), part.start_byte, part.len_byte))
            .collect::<Vec<_>>(),
        [("0", 0, 6), ("1", 6, 0), ("2", 6, 4)]
    );
    let native = &item.native.as_ref().unwrap().data;
    assert_eq!(native["summary_from_common"], true);
    assert!(
        native["item"]["summary"]
            .as_array()
            .unwrap()
            .iter()
            .all(|part| part.get("text").is_none())
    );
    assert_eq!(native["item"]["content"][0]["text"], "raw-private-fixture");
    assert_eq!(render(&item).unwrap(), original);
    let restored: CompletedModelItem =
        serde_json::from_value(serde_json::to_value(&item).unwrap()).unwrap();
    assert_eq!(render(&restored).unwrap(), original);
    for corrupt_index in ["-1", "9007199254740993"] {
        let mut corrupt = item.clone();
        let Some(ModelOutputItem::ReasoningSummary(summary)) = &mut corrupt.normalized else {
            unreachable!()
        };
        summary.parts[0].index = corrupt_index.to_owned().try_into().unwrap();
        assert!(corrupt.validate().is_ok()); // Common indexes are provider-neutral.
        assert_eq!(render(&corrupt), Err(ModelError::InvalidStream));
    }
}

#[test]
fn unknown_summary_arrays_remain_whole_native_and_old_native_only_records_replay_unchanged() {
    for summary in [
        json!([{"type":"summary_text","text":"readable"},{"type":"future_summary","text":"unknown"}]),
        json!([{"type":"summary_text","text":17}]),
        Value::Null,
        json!({"future":true}),
    ] {
        let original = json!({"type":"reasoning","id":"rs","summary":summary,"encrypted_content":"opaque-fixture"});
        let item = capture(&original, &scope(), None, None).unwrap();
        assert!(
            matches!(&item.normalized,Some(ModelOutputItem::ReasoningSummary(summary))if summary.text.is_empty()&&summary.parts.is_empty())
        );
        assert!(
            item.native
                .as_ref()
                .unwrap()
                .data
                .get("summary_from_common")
                .is_none()
        );
        assert_eq!(render(&item).unwrap(), original);
    }
    let original = json!({"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"legacy public body"}],"encrypted_content":"opaque-fixture"});
    let mut legacy = capture(&original, &scope(), None, None).unwrap();
    legacy.normalized = None;
    legacy.native.as_mut().unwrap().data = json!({"item":original,"text_bindings":[]});
    assert_eq!(render(&legacy).unwrap(), original);
}

#[test]
fn chat_rejects_public_reasoning_with_or_without_native_and_responses_replays_the_original() {
    use research_agent_core::model::ModelCapabilities;
    let original = json!({"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"public你"}],"encrypted_content":"opaque-fixture"});
    let item = capture(&original, &scope(), None, None).unwrap();
    for native in [true, false] {
        let mut item = item.clone();
        if !native {
            item.native = None;
        }
        assert_eq!(
            super::super::super::responses_chat_compatibility::validate(&item),
            Err(ModelError::UnsupportedCapability)
        );
        let request = ModelRequest {
            request_id: Uuid::new_v4(),
            captured_history: None,
            model: scope().model,
            messages: vec![ModelMessage::CanonicalOutput(Box::new(item))],
            tools: vec![],
            max_output_tokens: 1,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let value = super::super::super::responses_request::encode_responses_request(
            &request,
            ModelCapabilities {
                vision: false,
                tools: false,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
            super::super::super::responses_request::ResponsesEndpointKind::Platform,
        )
        .unwrap();
        assert_eq!(value["input"][0]["summary"], original["summary"]);
        if native {
            assert_eq!(value["input"][0], original);
        }
    }
}
