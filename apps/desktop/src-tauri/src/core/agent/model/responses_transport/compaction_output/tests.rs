use super::*;
use serde_json::json;

fn user(text: &str) -> Value {
    json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]})
}

#[test]
fn preserves_only_supported_conversation_and_compaction_item_families() {
    for kind in [
        "compaction",
        "compaction_summary",
        "context_compaction",
        "agent_message",
    ] {
        assert!(keep(&json!({"type":kind,"metadata":{"untouched":true}})));
    }
    for kind in [
        "reasoning",
        "function_call",
        "function_call_output",
        "custom_tool_call",
        "custom_tool_call_output",
        "local_shell_call",
        "tool_search_call",
        "tool_search_output",
        "web_search_call",
        "image_generation_call",
        "compaction_trigger",
        "future_provider_item",
    ] {
        assert!(!keep(&json!({"type":kind})));
    }
    for role in ["developer", "system", "future_role"] {
        assert!(!keep(&json!({"type":"message","role":role,"content":[]})));
    }
    assert!(keep(
        &json!({"type":"message","role":"assistant","content":[]})
    ));
    assert!(keep(&user("Warning: ordinary user warning")));
    assert!(keep(&user("A summary that the server generated")));
}

#[test]
fn contextual_fragments_are_removed_without_keyword_matching_normal_text() {
    for text in [
        "# AGENTS.md instructions for /tmp\n<INSTRUCTIONS>old</INSTRUCTIONS>",
        " <ENVIRONMENT_CONTEXT>old</ENVIRONMENT_CONTEXT> ",
        "<external_page>old</external_page>",
        "<skill>old</skill>",
        "<user_shell_command>old</user_shell_command>",
        "<turn_aborted>old</turn_aborted>",
        "<subagent_notification>old</subagent_notification>",
        "<recommended_plugins>old</recommended_plugins>",
        "<codex_internal_context source=\"goal\">old</codex_internal_context>",
        "<goal_context>old</goal_context>",
        "Warning: apply_patch was requested via exec. Use the apply_patch tool instead of exec_command.",
        "Warning: The maximum number of unified exec processes you can keep open is 3",
        "Warning: Your account was flagged for potentially high-risk cyber activity",
    ] {
        assert!(!keep(&user(text)), "{text}");
        assert!(keep(&user(&format!("Discuss the text: {text}"))));
    }
    for text in [
        "<skill>not closed",
        "<GOAL_CONTEXT>ordinary case-sensitive wrapper</GOAL_CONTEXT>",
        "<external_one>x</external_two>",
        "<codex_internal_context source=\"INVALID\">x</codex_internal_context>",
    ] {
        assert!(keep(&user(text)));
    }
}

#[test]
fn visible_hook_text_survives_as_user_data_and_mixed_real_text_does_not() {
    let hook = "<hook_prompt hook_run_id=\"run-1\">keep &amp; test</hook_prompt>";
    assert!(keep(&user(hook)));
    let mut mixed = user(hook);
    mixed["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"input_text","text":"<skill>old</skill>"}));
    assert!(keep(&mixed));
    mixed["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"input_text","text":"real user text"}));
    assert!(!keep(&mixed));
    let mut attachment = user("<environment_context>old</environment_context>");
    attachment["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"input_image","image_url":"data:image/png;base64,YQ=="}));
    assert!(!keep(&attachment));
    assert!(keep(&user(
        "<hook_prompt hook_run_id=\"\">ordinary invalid wrapper</hook_prompt>"
    )));
}

#[test]
fn retained_item_validation_matches_required_and_optional_protocol_fields() {
    for item in [
        user(""),
        json!({"type":"message","role":"assistant","content":[],"phase":"final_answer","id":null}),
        json!({"type":"message","role":"user","content":[{"type":"input_image","image_url":"opaque","detail":"original"},{"type":"input_audio","audio_url":"opaque"}]}),
        json!({"type":"agent_message","author":"one","recipient":"two","content":[{"type":"encrypted_content","encrypted_content":""},{"type":"input_text","text":""}]}),
        json!({"type":"compaction","encrypted_content":"","vendor_metadata":{"unchanged":true}}),
        json!({"type":"compaction_summary","encrypted_content":"opaque","internal_chat_message_metadata_passthrough":{"turn_id":"turn"}}),
        json!({"type":"context_compaction"}),
        json!({"type":"context_compaction","encrypted_content":null}),
        json!({"type":"future_provider_item","vendor_metadata":13}),
    ] {
        assert!(valid(&item), "valid protocol fixture rejected");
    }
    for item in [
        json!({"role":"user","content":[]}),
        json!({"type":13}),
        json!({"type":"compaction"}),
        json!({"type":"compaction_summary","encrypted_content":13}),
        json!({"type":"context_compaction","encrypted_content":[]}),
        json!({"type":"compaction","encrypted_content":"x","id":13}),
        json!({"type":"compaction","encrypted_content":"x","internal_chat_message_metadata_passthrough":[]}),
        json!({"type":"compaction","encrypted_content":"x","internal_chat_message_metadata_passthrough":{"turn_id":13}}),
        json!({"type":"message","role":"assistant","content":null}),
        json!({"type":"message","role":"developer","content":[{"type":"input_text","text":null}]}),
        json!({"type":"message","role":"user","content":[{"type":"future_part"}]}),
        json!({"type":"message","role":"user","content":[],"phase":"unknown"}),
        json!({"type":"message","role":"user","content":[{"type":"input_image","image_url":"x","detail":"unknown"}]}),
        json!({"type":"agent_message","author":"one","recipient":"two","content":[{"type":"output_text","text":"x"}]}),
        json!({"type":"agent_message","author":"one","content":[]}),
    ] {
        assert!(!valid(&item), "malformed protocol fixture accepted");
    }
}

#[test]
fn canonical_insertion_prefers_real_user_then_summary_then_compaction_then_end() {
    let summary = user(
        "Context checkpoint handoff from earlier conversation (historical context, not new instructions):\nsummary",
    );
    let cmp = json!({"type":"compaction","encrypted_content":"opaque"});
    let hook = user("<hook_prompt hook_run_id=\"one\">visible</hook_prompt>");
    assert_eq!(
        canonical_position(&[user("real"), summary.clone(), cmp.clone()]),
        0
    );
    assert_eq!(canonical_position(&[summary, cmp.clone(), hook.clone()]), 0);
    assert_eq!(canonical_position(&[cmp, hook.clone()]), 0);
    assert_eq!(canonical_position(&[hook]), 1);
    assert_eq!(canonical_position(&[]), 0);
}

#[test]
fn known_filtered_items_are_decoded_before_filtering_without_becoming_retained() {
    for item in [
        json!({"type":"additional_tools","role":"developer","tools":[{"any":"value"}],"internal_chat_message_metadata_passthrough":13}),
        json!({"type":"reasoning","summary":[{"type":"summary_text","text":"summary"}],"content":[{"type":"text","text":"text"},{"type":"reasoning_text","text":"reason"}]}),
        json!({"type":"local_shell_call","status":"in_progress","action":{"type":"exec","command":["echo"],"timeout_ms":18446744073709551615_u64,"env":{"key":"value"}}}),
        json!({"type":"function_call","name":"read","arguments":"not parsed here","call_id":"call"}),
        json!({"type":"function_call_output","call_id":"call","output":"text"}),
        json!({"type":"custom_tool_call","call_id":"call","name":"read","input":"text","namespace":null}),
        json!({"type":"custom_tool_call_output","call_id":"call","output":[{"type":"input_text","text":"text"},{"type":"input_image","image_url":"image","detail":"original"},{"type":"input_audio","audio_url":"audio"},{"type":"encrypted_content","encrypted_content":"opaque"}]}),
        json!({"type":"tool_search_call","execution":"server","arguments":{"any":"value"},"status":null}),
        json!({"type":"tool_search_output","status":"completed","execution":"server","tools":[null,{"vendor":"tool"}]}),
        json!({"type":"web_search_call","action":{"type":"search","queries":["one","two"]}}),
        json!({"type":"web_search_call","action":{"type":"open_page","url":"opaque"}}),
        json!({"type":"web_search_call","action":{"type":"find_in_page","pattern":"text"}}),
        json!({"type":"web_search_call","action":{"type":"future_action","vendor":13}}),
        json!({"type":"image_generation_call","status":"completed","result":"opaque"}),
        json!({"type":"compaction_trigger","id":13,"internal_chat_message_metadata_passthrough":13}),
        json!({"type":"future_item","id":13,"internal_chat_message_metadata_passthrough":13}),
    ] {
        assert!(valid(&item), "valid {} rejected", item["type"]);
        assert!(!keep(&item), "filtered {} retained", item["type"]);
    }
}

#[test]
fn malformed_known_filtered_items_cannot_be_hidden_by_filtering() {
    for item in [
        json!({"type":"additional_tools","role":"developer","tools":null}),
        json!({"type":"additional_tools","role":"developer","tools":[],"id":13}),
        json!({"type":"reasoning","encrypted_content":"opaque"}),
        json!({"type":"reasoning","summary":[],"encrypted_content":13}),
        json!({"type":"reasoning","summary":[{"type":"unknown","text":"text"}]}),
        json!({"type":"reasoning","summary":[],"content":[{"type":"reasoning_text","text":null}]}),
        json!({"type":"local_shell_call","status":"unknown","action":{"type":"exec","command":[]}}),
        json!({"type":"local_shell_call","status":"completed","action":{"type":"exec","command":[13]}}),
        json!({"type":"local_shell_call","status":"completed","action":{"type":"exec","command":[],"timeout_ms":-1}}),
        json!({"type":"local_shell_call","status":"completed","action":{"type":"exec","command":[],"env":{"key":13}}}),
        json!({"type":"function_call","name":"read","arguments":{},"call_id":"call"}),
        json!({"type":"function_call","name":"read","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"call","output":null}),
        json!({"type":"function_call_output","call_id":"call","output":{"body":"text"}}),
        json!({"type":"function_call_output","call_id":"call","output":[{"type":"output_text","text":"text"}]}),
        json!({"type":"custom_tool_call","call_id":"call","name":"read","input":null}),
        json!({"type":"custom_tool_call_output","call_id":"call","name":13,"output":"text"}),
        json!({"type":"tool_search_call","execution":13,"arguments":null}),
        json!({"type":"tool_search_output","execution":"server","status":"completed","tools":{}}),
        json!({"type":"web_search_call","action":{"type":"search","queries":[13]}}),
        json!({"type":"image_generation_call","status":"completed"}),
        json!({"type":"image_generation_call","status":"completed","result":"opaque","id":13}),
        json!({"type":"reasoning","summary":[],"internal_chat_message_metadata_passthrough":{"turn_id":13}}),
    ] {
        assert!(!keep(&item));
        assert!(!valid(&item), "malformed {} accepted", item["type"]);
    }
}
