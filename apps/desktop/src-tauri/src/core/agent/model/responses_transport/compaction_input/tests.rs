use super::*;

#[test]
fn ordinary_media_normalization_does_not_repair_or_remove_tool_pairs() {
    let original = json!({"input":[
        {"type":"function_call","call_id":"missing","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"orphan","output":"original result"},
        {"type":"message","role":"user","content":[{"type":"input_audio","audio_url":"audio"}]},
        {"type":"compaction","encrypted_content":"opaque","provider_extension":3}
    ]});
    let mut prompt = original.clone();
    normalize_restored_media(&mut prompt, true).unwrap();
    assert_eq!(prompt["input"].as_array().unwrap().len(), 4);
    for index in [0, 1, 3] {
        assert_eq!(prompt["input"][index], original["input"][index]);
    }
    assert_eq!(prompt["input"][2]["content"][0]["text"], AUDIO_OMITTED);
    assert_eq!(original["input"][2]["content"][0]["audio_url"], "audio");
}

#[test]
fn repairs_each_call_family_without_mutating_original_or_replacing_successful_results() {
    let original = json!({"input":[
        {"type":"function_call","id":"fc_one","call_id":"same","name":"read","arguments":"{}"},
        {"type":"custom_tool_call","id":"ctc_one","call_id":"same","input":"text"},
        {"type":"tool_search_call","id":"tsc_one","call_id":"search"},
        {"type":"local_shell_call","call_id":"shell","action":{"type":"exec"}},
        {"type":"function_call","call_id":"success","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"success","output":"original evidence","metadata":{"original":true}},
        {"type":"function_call_output","call_id":"orphan","output":"orphan evidence"},
        {"type":"tool_search_output","call_id":"server","execution":"server","tools":[{"name":"kept"}]},
        {"type":"tool_search_output","execution":"client","tools":[]},
        {"type":"compaction","encrypted_content":"unchanged","vendor_field":1},
        {"type":"compaction_trigger"}
    ]});
    let mut body = original.clone();
    normalize_body(&mut body, true).unwrap();
    let input = body["input"].as_array().unwrap();
    assert_eq!(input[1]["type"], "function_call_output");
    assert_eq!(input[1]["output"], "aborted");
    assert!(input[1]["id"].as_str().unwrap().starts_with("fco_"));
    assert_eq!(input[3]["type"], "custom_tool_call_output");
    assert_eq!(input[5]["tools"], json!([]));
    assert_eq!(input[7]["type"], "function_call_output");
    assert!(input[7].get("id").is_none());
    assert!(input.contains(&original["input"][5]));
    assert!(!input.contains(&original["input"][6]));
    for index in 7..=10 {
        assert!(input.contains(&original["input"][index]));
    }
    let mut again = original.clone();
    normalize_body(&mut again, true).unwrap();
    assert_eq!(
        body, again,
        "prompt-only output IDs are stable across attempts"
    );
    normalize_body(&mut again, true).unwrap();
    assert_eq!(body, again, "normalization is idempotent");
    assert_eq!(original["input"].as_array().unwrap().len(), 11);
}

#[test]
fn client_orphans_are_removed_but_server_search_and_unassociated_search_are_kept() {
    let mut body = json!({"input":[
        {"type":"function_call_output","call_id":"missing","output":"lost"},
        {"type":"custom_tool_call_output","call_id":"missing","output":"lost"},
        {"type":"tool_search_output","call_id":"missing","execution":"client","tools":[]},
        {"type":"tool_search_output","call_id":"missing","execution":"server","tools":[]},
        {"type":"tool_search_output","execution":"client","tools":[]}
    ]});
    normalize_body(&mut body, true).unwrap();
    assert_eq!(body["input"].as_array().unwrap().len(), 2);
    assert_eq!(body["input"][0]["execution"], "server");
    assert!(body["input"][1].get("call_id").is_none());
}

#[test]
fn unsupported_media_are_explicit_placeholders_and_other_fields_remain_opaque() {
    let original = json!({"input":[
        {"type":"message","role":"user","content":[{"type":"input_image","image_url":"fixture-image"},{"type":"input_audio","audio_url":"fixture-audio"},{"type":"input_text","text":"original"}],"metadata":7},
        {"type":"custom_tool_call","call_id":"media","input":"read"},
        {"type":"custom_tool_call_output","call_id":"media","output":[{"type":"input_image","image_url":"tool-image"},{"type":"input_audio","audio_url":"tool-audio"}],"success":false},
        {"type":"image_generation_call","result":"large image","status":"completed"},
        {"type":"future_provider_item","content":[{"type":"input_audio","data":"opaque"}]}
    ]});
    let mut body = original.clone();
    normalize_body(&mut body, false).unwrap();
    assert_eq!(body["input"][0]["metadata"], 7);
    assert_eq!(
        body["input"][0]["content"][2],
        original["input"][0]["content"][2]
    );
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert!(
        body["input"][0]["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("audio content omitted")
    );
    assert_eq!(body["input"][2]["output"][0]["type"], "input_text");
    assert_eq!(body["input"][2]["success"], false);
    assert_eq!(body["input"][3]["result"], "");
    assert_eq!(body["input"][4], original["input"][4]);
    let mut supported = original.clone();
    normalize_body(&mut supported, true).unwrap();
    assert_eq!(
        supported["input"][0]["content"][0],
        original["input"][0]["content"][0]
    );
    assert_eq!(supported["input"][3], original["input"][3]);
}
