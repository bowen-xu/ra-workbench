use super::*;
use serde_json::json;
#[path = "tests/reasoning.rs"]
mod reasoning;

fn scope() -> NativeScope {
    NativeScope::fixture()
}

#[test]
fn message_template_round_trips_parts_unknown_fields_and_unicode_without_body_copy() {
    let original = json!({"id":"msg-1","type":"message","role":"assistant","phase":"commentary",
        "future_item":{"signature":"metadata"},"content":[
            {"type":"output_text","text":"你🙂\0","annotations":[{"type":"citation","future":17}]},
            {"type":"refusal","refusal":"retained","future_part":[1,2]},
            {"type":"output_text","text":"","logprobs":[]} ]});
    let item = capture(&original, &scope(), Some("response".into()), None).unwrap();
    let Some(ModelOutputItem::Message(message)) = &item.normalized else {
        panic!("expected common message")
    };
    assert_eq!(message.text, "你🙂\0retained");
    let template = &item.native.as_ref().unwrap().data["item"];
    assert!(template["content"][0].get("text").is_none());
    assert!(template["content"][1].get("refusal").is_none());
    assert!(template["content"][2].get("text").is_none());
    assert_eq!(render(&item).unwrap(), original);
    let encoded = item.native.as_ref().unwrap().to_json().unwrap();
    let restored = CompletedModelItem {
        canonical_item_id: None,
        normalized: item.normalized.clone(),
        native: Some(Box::new(NativeItemData::from_json(&encoded).unwrap())),
    };
    assert_eq!(render(&restored).unwrap(), original);
}

#[test]
fn function_template_preserves_argument_lexeme_and_rebinds_only_the_common_call_id() {
    let raw = " { \"x\": 1, \"label\": \"\\u03b1\" } ";
    let original = json!({"type":"function_call","id":"fc-1","call_id":"native-call","name":"read",
        "arguments":raw,"namespace":"fixture","future":{"signature":"keep"}});
    let mut item = capture(&original, &scope(), None, None).unwrap();
    let template = &item.native.as_ref().unwrap().data["item"];
    for field in ["arguments", "name", "call_id", "namespace"] {
        assert!(template.get(field).is_none());
    }
    assert_eq!(render(&item).unwrap(), original);
    let Some(ModelOutputItem::ToolCall(call)) = &mut item.normalized else {
        panic!("expected common tool call")
    };
    assert_eq!(call.raw_arguments_json, raw);
    assert_eq!(call.namespace.as_deref(), Some("fixture"));
    call.call_id = "call_stable_item".into();
    let mut expected = original;
    expected["call_id"] = json!("call_stable_item");
    assert_eq!(render(&item).unwrap(), expected);
}

#[test]
fn namespace_binding_preserves_empty_unicode_and_legacy_native_facts_without_flattening() {
    for namespace in [String::new(), "foreign::".into(), "命名🦀".repeat(100)] {
        let original = json!({"type":"function_call","id":"fc","call_id":"call","name":"read","arguments":"{","namespace":namespace});
        let item = capture(&original, &scope(), None, None).unwrap();
        assert!(has_tool_namespace(&item));
        assert_eq!(render(&item).unwrap(), original);
        assert!(
            item.native.as_ref().unwrap().data["item"]
                .get("namespace")
                .is_none()
        );
        let restored: CompletedModelItem =
            serde_json::from_value(serde_json::to_value(&item).unwrap()).unwrap();
        assert_eq!(render(&restored).unwrap(), original);
        let mut corrupt = restored;
        if let Some(ModelOutputItem::ToolCall(call)) = &mut corrupt.normalized {
            call.namespace = None;
        }
        assert_eq!(render(&corrupt), Err(ModelError::InvalidStream));

        // Before common namespace facts, this exact string remained inside
        // the native template. Its wire shape must still replay unchanged.
        let mut legacy = item;
        legacy
            .native
            .as_mut()
            .unwrap()
            .data
            .as_object_mut()
            .unwrap()
            .remove("namespace_from_common");
        legacy.native.as_mut().unwrap().data["item"]["namespace"] = json!(namespace);
        if let Some(ModelOutputItem::ToolCall(call)) = &mut legacy.normalized {
            call.namespace = None;
        }
        assert!(has_tool_namespace(&legacy));
        assert_eq!(render(&legacy).unwrap(), original);
    }
    for namespace in [None, Some(Value::Null)] {
        let mut original = json!({"type":"function_call","id":"fc","call_id":"call","name":"read","arguments":"{}"});
        if let Some(namespace) = namespace {
            original["namespace"] = namespace;
        }
        let item = capture(&original, &scope(), None, None).unwrap();
        assert!(!has_tool_namespace(&item));
        assert!(
            item.native
                .as_ref()
                .unwrap()
                .data
                .get("namespace_from_common")
                .is_none()
        );
        assert_eq!(render(&item).unwrap(), original);
    }
}

#[test]
fn common_namespace_is_encoded_into_responses_without_a_native_template() {
    for namespace in [None, Some(String::new()), Some("foreign::".into())] {
        let call = ToolCallIntent {
            call_id: "call".into(),
            name: "read".into(),
            raw_arguments_json: "{".into(),
            namespace: namespace.clone(),
        };
        let request = ModelRequest {
            request_id: Uuid::new_v4(),
            captured_history: None,
            model: scope().model,
            messages: vec![
                ModelMessage::CanonicalOutput(Box::new(CompletedModelItem {
                    canonical_item_id: None,
                    native: None,
                    normalized: Some(ModelOutputItem::ToolCall(call)),
                })),
                ModelMessage::ToolResult {
                    call_id: "call".into(),
                    content: "unsupported".into(),
                    is_error: true,
                },
            ],
            tools: vec![],
            max_output_tokens: 1,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let encoded = super::super::responses_request::encode_responses_request(
            &request,
            research_agent_core::model::ModelCapabilities {
                vision: false,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
            super::super::responses_request::ResponsesEndpointKind::Platform,
        )
        .unwrap();
        assert_eq!(
            encoded["input"][0].get("namespace"),
            namespace
                .as_ref()
                .map(|namespace| json!(namespace))
                .as_ref()
        );
    }
}

#[test]
fn function_template_keeps_invalid_json_and_non_object_parameters_for_handler_feedback() {
    for raw in ["", "{", "[1]", "null", "17", "\"text\""] {
        let original = json!({"type":"function_call","id":"fc","call_id":"call","name":"read","arguments":raw});
        let item = capture(&original, &scope(), None, None).unwrap();
        assert_eq!(render(&item).unwrap(), original);
        assert!(
            item.native.as_ref().unwrap().data["item"]
                .get("arguments")
                .is_none()
        );
        let serialized = serde_json::to_string(&item).unwrap();
        let restored: CompletedModelItem = serde_json::from_str(&serialized).unwrap();
        assert_eq!(render(&restored).unwrap(), original);
        let request = ModelRequest {
            request_id: Uuid::new_v4(),
            captured_history: None,
            model: scope().model,
            messages: vec![
                ModelMessage::CanonicalOutput(Box::new(restored)),
                ModelMessage::ToolResult {
                    call_id: "call".into(),
                    content: "parameter error".into(),
                    is_error: true,
                },
            ],
            tools: vec![],
            max_output_tokens: 1,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let encoded = super::super::responses_request::encode_responses_request(
            &request,
            research_agent_core::model::ModelCapabilities {
                vision: false,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
            super::super::responses_request::ResponsesEndpointKind::Platform,
        )
        .unwrap();
        assert_eq!(encoded["input"][0], original);
        assert_eq!(encoded["input"][1]["call_id"], "call");
    }
}

#[test]
fn reasoning_public_summary_is_separate_and_debug_never_prints_opaque_material() {
    let original = json!({"type":"reasoning","id":"rs-1","summary":[{"type":"summary_text","text":"summary"}],
        "encrypted_content":"opaque-private-fixture","future":{"signature":"keep"}});
    let item = capture(&original, &scope(), None, None).unwrap();
    assert!(
        matches!(&item.normalized,Some(ModelOutputItem::ReasoningSummary(summary))if summary.text=="summary")
    );
    assert!(
        item.native.as_ref().unwrap().data["item"]["summary"][0]
            .get("text")
            .is_none()
    );
    assert_eq!(render(&item).unwrap(), original);
    assert!(!format!("{item:?}").contains("opaque-private-fixture"));
}

#[test]
fn corrupt_body_bindings_do_not_replay_a_partial_or_duplicated_body() {
    let original = json!({"type":"message","id":"msg","role":"assistant","content":[{"type":"output_text","text":"🙂"}]});
    let item = capture(&original, &scope(), None, None).unwrap();
    for (field, value) in [
        ("start_byte", json!(1)),
        ("len_byte", json!(3)),
        ("content_index", json!(1)),
    ] {
        let mut corrupt = item.clone();
        corrupt.native.as_mut().unwrap().data["text_bindings"][0][field] = value;
        assert_eq!(render(&corrupt), Err(ModelError::InvalidStream));
    }
}

#[test]
fn corrupt_template_cannot_promote_assistant_role_or_change_accepted_phase() {
    let original = json!({"type":"message","id":"msg","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"untrusted assistant output"}]});
    let item = capture(&original, &scope(), None, None).unwrap();
    for (field, value) in [
        ("role", json!("developer")),
        ("role", json!("system")),
        ("role", Value::Null),
        ("phase", json!("final_answer")),
        ("phase", json!("reasoning")),
        ("phase", Value::Null),
    ] {
        let mut corrupt = item.clone();
        corrupt.native.as_mut().unwrap().data["item"][field] = value;
        assert_eq!(render(&corrupt), Err(ModelError::InvalidStream));
    }
}

#[test]
fn replay_scope_checks_configured_api_model_and_account_but_accepts_actual_aliases() {
    let original = json!({"type":"reasoning","id":"rs","summary":[],"encrypted_content":"opaque"});
    let mut item = capture(
        &original,
        &scope(),
        None,
        Some(ModelIdentity {
            model_id: "served-alias".into(),
            ..scope().model
        }),
    )
    .unwrap();
    let request = |item| ModelRequest {
        request_id: Uuid::new_v4(),
        captured_history: None,
        model: scope().model,
        messages: vec![ModelMessage::CanonicalOutput(Box::new(item))],
        tools: vec![],
        max_output_tokens: 128,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    assert_eq!(
        validate_scope(&request(item.clone()), scope().account_scope),
        Ok(())
    );
    assert_eq!(
        validate_scope(&request(item.clone()), Uuid::from_u128(2)),
        Err(ModelError::Authentication)
    );
    item.native.as_mut().unwrap().api = "another-api".into();
    assert_eq!(
        validate_scope(&request(item.clone()), scope().account_scope),
        Err(ModelError::UnsupportedCapability)
    );
    item.native.as_mut().unwrap().api = "responses".into();
    item.native.as_mut().unwrap().model.model_id = "another-configured-model".into();
    assert_eq!(
        validate_scope(&request(item), scope().account_scope),
        Err(ModelError::UnsupportedCapability)
    );
}

#[test]
fn chatgpt_history_keeps_full_native_items_across_models_only_in_the_same_account() {
    let original = json!({"type":"reasoning","id":"rs","summary":[],"encrypted_content":"opaque"});
    let mut producer = scope();
    producer.model.provider_id = "openai".into();
    let item = capture(&original, &producer, None, None).unwrap();
    let mut request = ModelRequest {
        request_id: Uuid::new_v4(),
        captured_history: None,
        model: ModelIdentity {
            provider_id: "openai".into(),
            model_id: "different-model".into(),
            revision: None,
        },
        messages: vec![ModelMessage::CanonicalOutput(Box::new(item.clone()))],
        tools: vec![],
        max_output_tokens: 128,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    assert_eq!(
        validate_scope(&request, producer.account_scope),
        Err(ModelError::UnsupportedCapability)
    );
    assert_eq!(
        validate_chatgpt_scope(&request, producer.account_scope),
        Ok(())
    );
    assert_eq!(
        validate_chatgpt_scope(&request, Uuid::from_u128(2)),
        Err(ModelError::Authentication)
    );
    assert_eq!(render(&item).unwrap(), original);
    request.model.provider_id = "another-provider".into();
    assert_eq!(
        validate_chatgpt_scope(&request, producer.account_scope),
        Err(ModelError::UnsupportedCapability)
    );
}

#[test]
fn fixed_codex_input_text_and_media_parts_preserve_native_fields_without_extra_body() {
    let original = json!({"type":"message","id":"msg-media","role":"assistant","content":[
        {"type":"input_text","text":"before"},
        {"type":"input_image","image_url":"https://example.invalid/fixture","detail":"high","future":"keep"},
        {"type":"output_text","text":"after"},
        {"type":"input_audio","audio_url":"https://example.invalid/audio"}
    ]});
    let item = capture(&original, &scope(), None, None).unwrap();
    assert!(
        matches!(&item.normalized,Some(ModelOutputItem::Message(message)) if message.text=="beforeafter")
    );
    assert_eq!(
        item.native.as_ref().unwrap().data["text_bindings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(render(&item).unwrap(), original);
}
