//! Prompt-only normalization migrated from Codex context_manager/normalize.rs,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
//! Run after native window expansion, never write repaired items to the journal.
use super::*;
use serde_json::json;
use std::collections::HashSet;

#[path = "compaction_input/budget.rs"]
pub(super) mod budget;

const SYNTHETIC_NAMESPACE: Uuid = Uuid::from_u128(0x90d38d3e_6a5b_4d52_bfe2_2f1e634bfac4);
const IMAGE_OMITTED: &str = "image content omitted because you do not support image input";
const AUDIO_OMITTED: &str = "audio content omitted because you do not support audio input";

/// Normalized source attachments must be stripped before ordinary codec
/// validation. This clone is used only by compact, never ordinary sampling.
pub(super) fn prepare_request(request: &ModelRequest, vision: bool) -> ModelRequest {
    use research_agent_core::model::{ContentPart, ModelMessage};
    let mut prepared = request.clone();
    if !vision {
        for message in &mut prepared.messages {
            let parts = match message {
                ModelMessage::User { content: parts, .. }
                | ModelMessage::Assistant { content: parts, .. } => parts,
                _ => continue,
            };
            for part in parts {
                if matches!(part, ContentPart::Image { .. }) {
                    *part = ContentPart::Text(IMAGE_OMITTED.into());
                }
            }
        }
    }
    prepared
}

pub(super) fn normalize_body(body: &mut Value, vision: bool) -> Result<(), ModelError> {
    let input = body["input"]
        .as_array_mut()
        .ok_or(ModelError::InvalidRequest)?;
    let mut outputs = HashSet::new();
    let mut calls = HashSet::new();
    for item in input.iter() {
        if let Some(id) = item["call_id"].as_str() {
            if let Some(kind) = output_kind(item["type"].as_str()) {
                outputs.insert((kind, id.to_owned()));
            }
            if let Some(kind) = call_output_kind(item["type"].as_str()) {
                calls.insert((kind, id.to_owned()));
            }
        }
    }
    let mut normalized = Vec::with_capacity(input.len());
    for item in input.iter() {
        if let (Some(kind), Some(id)) = (output_kind(item["type"].as_str()), item["call_id"].as_str())
            && !calls.contains(&(kind, id.to_owned()))
            // Upstream server tool-search results need no client call pair.
            && !(kind == "tool_search_output" && item["execution"] == "server")
        {
            continue;
        }
        normalized.push(item.clone());
        if let (Some(kind), Some(id)) = (
            call_output_kind(item["type"].as_str()),
            item["call_id"].as_str(),
        ) && !outputs.contains(&(kind, id.to_owned()))
        {
            normalized.push(missing_output(kind, id, item["id"].as_str()));
        }
    }
    for item in &mut normalized {
        normalize_media(item, vision);
    }
    *input = normalized;
    Ok(())
}

/// Ordinary requests still validate current typed attachments before expansion.
/// Normalize only this expanded prompt copy; never change the installed window
/// or repair tool pairs as a side effect of ordinary sampling.
pub(super) fn normalize_restored_media(body: &mut Value, vision: bool) -> Result<(), ModelError> {
    let input = body["input"]
        .as_array_mut()
        .ok_or(ModelError::InvalidRequest)?;
    for item in input {
        normalize_media(item, vision);
    }
    Ok(())
}

fn output_kind(kind: Option<&str>) -> Option<&'static str> {
    match kind {
        Some("function_call_output") => Some("function_call_output"),
        Some("custom_tool_call_output") => Some("custom_tool_call_output"),
        Some("tool_search_output") => Some("tool_search_output"),
        _ => None,
    }
}
fn call_output_kind(kind: Option<&str>) -> Option<&'static str> {
    match kind {
        Some("function_call" | "local_shell_call") => Some("function_call_output"),
        Some("custom_tool_call") => Some("custom_tool_call_output"),
        Some("tool_search_call") => Some("tool_search_output"),
        _ => None,
    }
}
fn missing_output(kind: &str, call_id: &str, source_id: Option<&str>) -> Value {
    let (prefix, mut output) = match kind {
        "tool_search_output" => (
            "tso",
            json!({"type":kind,"call_id":call_id,"status":"completed","execution":"client","tools":[]}),
        ),
        "custom_tool_call_output" => (
            "ctco",
            json!({"type":kind,"call_id":call_id,"output":"aborted"}),
        ),
        _ => (
            "fco",
            json!({"type":kind,"call_id":call_id,"output":"aborted"}),
        ),
    };
    if let Some(id) = source_id.filter(|id| !id.is_empty()) {
        let suffix = Uuid::new_v5(&SYNTHETIC_NAMESPACE, format!("{prefix}:{id}").as_bytes());
        output["id"] = json!(format!("{prefix}_{suffix}"));
    }
    output
}
fn normalize_media(item: &mut Value, vision: bool) {
    let parts = match item["type"].as_str() {
        Some("message") | None if item.get("role").is_some() => item.get_mut("content"),
        Some("function_call_output" | "custom_tool_call_output") => item.get_mut("output"),
        Some("image_generation_call") if !vision => {
            item["result"] = json!("");
            None
        }
        _ => None,
    };
    if let Some(parts) = parts.and_then(Value::as_array_mut) {
        for part in parts {
            let reason = match part["type"].as_str() {
                Some("input_image") if !vision => Some(IMAGE_OMITTED),
                // Current native model contract has no audio input capability.
                Some("input_audio") => Some(AUDIO_OMITTED),
                _ => None,
            };
            if let Some(text) = reason {
                *part = json!({"type":"input_text","text":text});
            }
        }
    }
}

#[cfg(test)]
#[path = "compaction_input/tests.rs"]
mod tests;
