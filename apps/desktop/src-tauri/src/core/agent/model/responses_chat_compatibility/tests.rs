use super::*;
use crate::agent::model::responses_native::{NativeScope, capture, render};
use research_agent_core::model::{AssistantMessage, ToolCallIntent};
use serde_json::json;

fn captured(original: Value) -> CompletedModelItem {
    capture(&original, &NativeScope::fixture(), None, None).unwrap()
}

fn message(parts: Value) -> Value {
    json!({"type":"message","id":"message","role":"assistant","content":parts})
}

fn call() -> Value {
    json!({"type":"function_call","id":"tool","call_id":"call",
        "name":"read","arguments":" { invalid raw JSON\n"})
}

fn unchanged(item: &CompletedModelItem, expected: Result<(), ModelError>) {
    let before = serde_json::to_value(item).unwrap();
    assert_eq!(validate(item), expected);
    assert_eq!(serde_json::to_value(item).unwrap(), before);
}

#[test]
fn common_history_keeps_existing_text_and_raw_intent_projection() {
    for normalized in [
        ModelOutputItem::Message(AssistantMessage {
            item_id: "plain".into(),
            text: "Plain refusal words remain ordinary legacy text.".into(),
            phase: None,
        }),
        ModelOutputItem::ToolCall(ToolCallIntent {
            call_id: "call".into(),
            name: "read".into(),
            raw_arguments_json: " { invalid raw JSON\n".into(),
            namespace: None,
        }),
    ] {
        unchanged(
            &CompletedModelItem {
                canonical_item_id: None,
                normalized: Some(normalized),
                native: None,
            },
            Ok(()),
        );
    }
}

#[test]
fn proven_text_and_flat_tool_records_are_compatible_without_rewriting() {
    let mut original = message(json!([
        {"type":"input_text","text":"中文\u{0000}","annotations":[]},
        {"type":"output_text","text":"é🙂","logprobs":null}
    ]));
    original["phase"] = json!("commentary");
    original["status"] = json!("completed");
    for original in [original, call()] {
        let item = captured(original.clone());
        unchanged(&item, Ok(()));
        assert_eq!(render(&item).unwrap(), original);
    }
}

#[test]
fn native_media_and_refusal_never_fall_back_to_readable_text_subset() {
    for part in [
        json!({"type":"input_image","image_url":"fixture://image","detail":"high"}),
        json!({"type":"input_audio","audio_url":"fixture://audio"}),
        json!({"type":"refusal","refusal":"declined"}),
    ] {
        for prefix in [false, true] {
            let mut parts = Vec::new();
            if prefix {
                parts.push(json!({"type":"output_text","text":"readable prefix"}));
            }
            parts.push(part.clone());
            unchanged(
                &captured(message(Value::Array(parts))),
                Err(ModelError::UnsupportedCapability),
            );
        }
    }
}

#[test]
fn material_not_proven_representable_is_rejected_at_every_native_layer() {
    let plain = message(json!([{"type":"output_text","text":"readable"}]));
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/unrecognized_semantics", json!("retained")),
        ("/status", json!({"semantic":"status"})),
        (
            "/content/0/unrecognized_semantics",
            json!({"retained":true}),
        ),
        (
            "/content/0/annotations",
            json!([{"type":"citation","url":"fixture://source"}]),
        ),
        (
            "/content/0/logprobs",
            json!([{"token":"readable","logprob":-1}]),
        ),
    ] {
        let mut original = plain.clone();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        original
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(field.into(), value);
        cases.push(captured(original));
    }
    let mut envelope_extra = captured(plain.clone());
    envelope_extra
        .native
        .as_mut()
        .unwrap()
        .extra
        .insert("future_semantics".into(), json!(true));
    cases.push(envelope_extra);
    let mut template_extra = captured(plain.clone());
    template_extra.native.as_mut().unwrap().data["future_semantics"] = json!(true);
    cases.push(template_extra);
    let mut binding_extra = captured(plain);
    binding_extra.native.as_mut().unwrap().data["text_bindings"][0]["future_semantics"] =
        json!(true);
    cases.push(binding_extra);
    let mut tool_extra = call();
    tool_extra["future_semantics"] = json!("retained");
    cases.push(captured(tool_extra));
    for item in cases {
        unchanged(&item, Err(ModelError::UnsupportedCapability));
    }
}

#[test]
fn opaque_native_items_and_unknown_adapters_are_not_projected() {
    let reasoning = captured(json!({"type":"reasoning","id":"reasoning",
        "summary":[],"encrypted_content":"synthetic-opaque"}));
    unchanged(&reasoning, Err(ModelError::UnsupportedCapability));
    for field in ["api", "item_type"] {
        let mut item = captured(call());
        let native = item.native.as_mut().unwrap();
        match field {
            "api" => native.api = "future-api".into(),
            "item_type" => native.item_type = "future-item".into(),
            _ => unreachable!(),
        }
        unchanged(&item, Err(ModelError::UnsupportedCapability));
    }
}

#[test]
fn namespace_is_rejected_in_common_new_native_and_legacy_native_forms() {
    for namespace in ["", "foreign"] {
        let mut original = call();
        original["namespace"] = json!(namespace);
        let native = captured(original);
        unchanged(&native, Err(ModelError::UnsupportedCapability));
        let mut common = native.clone();
        common.native = None;
        unchanged(&common, Err(ModelError::UnsupportedCapability));

        let mut legacy = native;
        let Some(ModelOutputItem::ToolCall(intent)) = &mut legacy.normalized else {
            panic!("fixture tool intent");
        };
        intent.namespace = None;
        let data = &mut legacy.native.as_mut().unwrap().data;
        data.as_object_mut()
            .unwrap()
            .remove("namespace_from_common");
        data["item"]["namespace"] = json!(namespace);
        unchanged(&legacy, Err(ModelError::UnsupportedCapability));
    }
}

#[test]
fn corrupt_known_bindings_and_identity_remain_invalid_streams() {
    let plain = captured(message(json!([{"type":"output_text","text":"中文"}])));
    for (field, value) in [
        ("content_index", json!(1)),
        ("start_byte", json!(1)),
        ("len_byte", json!(1)),
        ("field", json!("refusal")),
    ] {
        let mut item = plain.clone();
        item.native.as_mut().unwrap().data["text_bindings"][0][field] = value;
        unchanged(&item, Err(ModelError::InvalidStream));
    }
    for (field, value) in [("role", json!("developer")), ("phase", json!("commentary"))] {
        let mut item = plain.clone();
        item.native.as_mut().unwrap().data["item"][field] = value;
        unchanged(&item, Err(ModelError::InvalidStream));
    }
}

#[test]
fn text_compatibility_proof_matches_every_part_and_the_complete_common_body() {
    let original = message(json!([
        {"type":"input_text","text":"中"},
        {"type":"output_text","text":"文🙂\u{0000}"},
        {"type":"output_text","text":""}
    ]));
    assert!(plain_message(&original, "中文🙂\u{0000}"));
    for common in ["", "中文", "中文🙂", "中文🙂\u{0000}extra", "中A🙂\u{0000}"] {
        assert!(!plain_message(&original, common));
    }
    assert!(!plain_message(&message(json!([])), "extra"));

    let mut item = captured(original);
    let Some(ModelOutputItem::Message(message)) = &mut item.normalized else {
        panic!("fixture message");
    };
    message.text = "中A".into();
    unchanged(&item, Err(ModelError::InvalidStream));
}

#[test]
fn known_template_keys_do_not_hide_malformed_or_unused_material() {
    for original in [message(json!([])), call()] {
        let baseline = captured(original);
        for value in [
            json!({"opaque":true}),
            json!("opaque"),
            json!(1),
            json!(false),
        ] {
            let mut item = baseline.clone();
            let native = item.native.as_mut().unwrap();
            native.data["text_bindings"] = value;
            assert!(!known_template(&native.data, &native.item_type));
            unchanged(&item, Err(ModelError::InvalidStream));
        }
        for value in [
            json!({"opaque":true}),
            json!("opaque"),
            json!(1),
            json!([]),
            Value::Null,
        ] {
            let mut item = baseline.clone();
            let native = item.native.as_mut().unwrap();
            native.data["namespace_from_common"] = value;
            assert!(!known_template(&native.data, &native.item_type));
            unchanged(&item, Err(ModelError::InvalidStream));
        }
        // The compatibility shape allows absent/empty bindings. The existing
        // native codec still owns detailed validity, including rejecting null.
        let mut absent = baseline.clone();
        absent
            .native
            .as_mut()
            .unwrap()
            .data
            .as_object_mut()
            .unwrap()
            .remove("text_bindings");
        unchanged(&absent, Ok(()));
        let mut null = baseline.clone();
        null.native.as_mut().unwrap().data["text_bindings"] = Value::Null;
        unchanged(&null, Err(ModelError::InvalidStream));
        let mut wrong_use = baseline;
        wrong_use.native.as_mut().unwrap().data["namespace_from_common"] = json!(true);
        unchanged(&wrong_use, Err(ModelError::InvalidStream));
    }
    let mut call = captured(call());
    let native = call.native.as_mut().unwrap();
    native.data["text_bindings"] = json!([
        {"content_index":0,"field":"text","start_byte":0,"len_byte":0}
    ]);
    assert!(!known_template(&native.data, &native.item_type));
    unchanged(&call, Err(ModelError::InvalidStream));
}
