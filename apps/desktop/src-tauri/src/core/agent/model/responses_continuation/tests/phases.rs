use super::*;

#[tokio::test]
async fn ordered_messages_and_native_phases_survive_checkpoint_restore() {
    let account = Uuid::from_u128(1);
    let scope = Uuid::new_v4();
    let state = ResponsesContinuation::new(scope, model(), 32_768).unwrap();
    let original = input();
    assert_eq!(
        state
            .prepare_sync(account, model(), original.clone())
            .unwrap(),
        original
    );
    let output = vec![
        json!({"type":"message","id":"m1","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Inspecting"}]}),
        json!({"type":"function_call","id":"f1","call_id":"call-1","name":"read","arguments":"{}"}),
        json!({"type":"message","id":"m2","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Waiting for source"}]}),
    ];
    let normalized_output = project_output(&output).unwrap();
    assert_eq!(normalized_output[0]["phase"], "commentary");
    assert_eq!(normalized_output[1]["type"], "function_call");
    assert_eq!(
        normalized_output[2]["content"][0]["text"],
        "Waiting for source"
    );
    state
        .preserve(receipt(original.clone(), original.clone(), output.clone()))
        .await
        .unwrap();
    let encoded = state.encode_checkpoint().unwrap();
    let restored = ResponsesContinuation::restore(scope, model(), 32_768, &encoded).unwrap();
    let result = json!({"type":"function_call_output","call_id":"call-1","output":"source"});
    let mut normalized = original.clone();
    normalized.extend(normalized_output);
    normalized.push(result.clone());
    let wire = restored
        .prepare_sync(account, model(), normalized.clone())
        .unwrap();
    let mut expected = original;
    expected.extend(output);
    expected.push(result);
    assert_eq!(wire, expected);
    normalized[1]["phase"] = json!("final_answer");
    assert_eq!(
        restored.prepare_sync(account, model(), normalized),
        Err(ModelError::InvalidRequest)
    );
}

#[test]
fn unknown_message_phase_stays_unknown_in_normalized_history() {
    let output = vec![
        json!({"type":"message","id":"m1","role":"assistant","phase":"future_phase","content":[{"type":"output_text","text":"text"}]}),
    ];
    let projected = project_output(&output).unwrap();
    assert!(projected[0].get("phase").is_none());
    assert_eq!(projected[0]["content"][0]["text"], "text");
}

#[test]
fn durable_results_reconcile_calls_with_interleaved_commentary() {
    let scope = Uuid::new_v4();
    let call = json!({"type":"function_call","call_id":"native","name":"read","arguments":"{}"});
    let commentary = json!({"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Reading"}]});
    let result = json!({"type":"function_call_output","call_id":"native","output":"source"});
    let encoded = json!({"schema_version":2,"scope_id":scope,"model":model(),"credential_scope":Uuid::new_v4(),
        "checkpoint":{"request_id":Uuid::new_v4(),"actual_model":model(),
            "normalized_prefix":[call,commentary],"wire_prefix":[call,commentary]}}).to_string();
    let completed = ResponsesContinuation::restore(scope, model(), 16_384, &encoded)
        .unwrap()
        .with_durable_tool_results(std::slice::from_ref(&result))
        .unwrap();
    let completed: Value = serde_json::from_str(&completed.encode_checkpoint().unwrap()).unwrap();
    assert_eq!(completed["checkpoint"]["wire_prefix"][1], commentary);
    assert_eq!(completed["checkpoint"]["wire_prefix"][2], result);
    let mut invalid: Value = serde_json::from_str(&encoded).unwrap();
    invalid["checkpoint"]["normalized_prefix"][1]["role"] = json!("user");
    assert!(
        ResponsesContinuation::restore(scope, model(), 16_384, &invalid.to_string())
            .unwrap()
            .with_durable_tool_results(&[result])
            .is_err()
    );
}

#[tokio::test]
async fn added_only_phase_continues_after_decoder_and_checkpoint_restore() {
    use crate::agent::model::responses_decoder::ResponsesDecoder;
    let identity = model();
    let account = Uuid::from_u128(1);
    let scope = Uuid::new_v4();
    let mut decoder = ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope {
            model: identity.clone(),
            account_scope: account,
        },
        32_768,
        2,
    )
    .unwrap();
    let final_item = json!({"id":"m1","type":"message","role":"assistant","content":[{"type":"output_text","text":"Inspecting"}]});
    let mut added_item = final_item.clone();
    added_item["phase"] = json!("commentary");
    for event in [
        json!({"type":"response.created","response":{"id":"r1","model":identity.model_id}}),
        json!({"type":"response.output_item.added","item":added_item}),
        json!({"type":"response.output_item.done","item":final_item}),
        json!({"type":"response.completed","response":{"id":"r1","model":identity.model_id,"status":"completed","end_turn":false,"output":[final_item]}}),
    ] {
        decoder.decode(&event.to_string()).unwrap();
    }
    let output = decoder.take_completed_output().unwrap();
    assert_eq!(output[0]["phase"], "commentary");
    let original = input();
    let state = ResponsesContinuation::new(scope, model(), 32_768).unwrap();
    state
        .prepare_sync(account, model(), original.clone())
        .unwrap();
    state
        .preserve(receipt(original.clone(), original.clone(), output.clone()))
        .await
        .unwrap();
    let restored =
        ResponsesContinuation::restore(scope, model(), 32_768, &state.encode_checkpoint().unwrap())
            .unwrap();
    let message = json!({"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Inspecting"}]});
    let mut normalized = original.clone();
    normalized.push(message);
    let expected = [original, output].concat();
    assert_eq!(
        restored.prepare_sync(account, model(), normalized).unwrap(),
        expected
    );
}

#[tokio::test]
async fn sparse_codex_done_items_continue_with_their_resolved_ids_after_reopen() {
    use crate::agent::model::responses_decoder::ResponsesDecoder;
    let account = Uuid::from_u128(1);
    let scope = Uuid::new_v4();
    let mut decoder = ResponsesDecoder::new(
        crate::agent::model::responses_native::NativeScope {
            model: model(),
            account_scope: Uuid::from_u128(1),
        },
        32_768,
        2,
    )
    .unwrap();
    for text in ["Inspecting", "Ready"] {
        decoder
            .decode(
                &json!({"type":"response.output_item.done", "item":{
                    "type":"message","role":"assistant","phase":"commentary",
                    "content":[{"type":"output_text","text":text}]
                }})
                .to_string(),
            )
            .unwrap();
    }
    decoder
        .decode(&json!({"type":"response.completed","response":{"id":"r1"}}).to_string())
        .unwrap();
    let output = decoder.take_completed_output().unwrap();
    assert_ne!(output[0]["id"], output[1]["id"]);
    assert!(
        output
            .iter()
            .all(|item| item["id"].as_str().unwrap().starts_with("msg_"))
    );
    let original = input();
    let state = ResponsesContinuation::new(scope, model(), 32_768).unwrap();
    state
        .prepare_sync(account, model(), original.clone())
        .unwrap();
    state
        .preserve(receipt(original.clone(), original.clone(), output.clone()))
        .await
        .unwrap();
    let restored =
        ResponsesContinuation::restore(scope, model(), 32_768, &state.encode_checkpoint().unwrap())
            .unwrap();
    let mut normalized = original.clone();
    normalized.extend(project_output(&output).unwrap());
    assert_eq!(
        restored.prepare_sync(account, model(), normalized).unwrap(),
        [original, output].concat()
    );
}
