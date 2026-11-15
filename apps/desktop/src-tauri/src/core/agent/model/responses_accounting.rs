//! OpenAI Responses compaction length heuristic from Codex context_manager/history.rs,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Copyright 2025 OpenAI, Apache-2.0.
//! This estimates opaque model-visible state; it does not decrypt or prove fidelity.
use research_agent_core::{
    context::{
        accounting::{ContextTokenEstimator, Utf8JsonTokenEstimator},
        compaction_strategy::CompactionOutput,
    },
    model::{ModelMessage, ToolDefinition},
};

pub(crate) struct ResponsesContextEstimator;
impl ContextTokenEstimator for ResponsesContextEstimator {
    fn estimate(&self, messages: &[ModelMessage], tools: &[ToolDefinition]) -> Option<u64> {
        Utf8JsonTokenEstimator.estimate(messages, tools)
    }
    fn text_byte_budget(&self, tokens: u64) -> Option<usize> {
        Utf8JsonTokenEstimator.text_byte_budget(tokens)
    }
    fn estimate_compaction(&self, output: &CompactionOutput) -> Option<u64> {
        match output {
            CompactionOutput::TextSummary(_) | CompactionOutput::ResetWindow => Some(0),
            CompactionOutput::RemoteWindow(items) => items.iter().try_fold(0u64, |total, item| {
                total.checked_add(wire_item_tokens(item)?)
            }),
            CompactionOutput::RemoteOpaque(item)
                if matches!(
                    item["type"].as_str(),
                    Some("compaction" | "compaction_summary")
                ) =>
            {
                let encoded_len = item["encrypted_content"].as_str()?.len();
                let visible = encoded_len.saturating_mul(3) / 4;
                u64::try_from(visible.saturating_sub(650).div_ceil(4)).ok()
            }
            _ => None,
        }
    }
}

fn wire_item_tokens(item: &serde_json::Value) -> Option<u64> {
    use serde_json::Value;
    match item["type"].as_str() {
        Some("compaction" | "compaction_summary") => {
            return ResponsesContextEstimator
                .estimate_compaction(&CompactionOutput::RemoteOpaque(item.clone()));
        }
        Some(
            "message"
            | "function_call"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
            | "local_shell_call"
            | "tool_search_call"
            | "tool_search_output"
            | "reasoning",
        ) => {}
        None if item["role"].is_string() => {}
        _ => return None,
    }
    // Binary input costs stay unknown until an explicit modality policy exists.
    for key in ["content", "output"] {
        if item[key].as_array().is_some_and(|parts| {
            parts
                .iter()
                .any(|part| matches!(part["type"].as_str(), Some("input_image" | "input_audio")))
        }) {
            return None;
        }
    }
    let mut text = item.clone();
    let extra = if let Some(encrypted) = item.get("encrypted_content") {
        let len = encrypted.as_str()?.len();
        text["encrypted_content"] = Value::String(String::new());
        u64::try_from((len.saturating_mul(3) / 4).saturating_sub(650).div_ceil(4)).ok()?
    } else {
        0
    };
    let bytes = serde_json::to_vec(&text).ok()?;
    u64::try_from(bytes.len())
        .ok()?
        .div_ceil(4)
        .checked_add(extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn opaque_heuristic_has_saturating_overhead_and_does_not_count_json_as_text() {
        for (len, tokens) in [(0, 0), (867, 0), (872, 1), (12000, 2088)] {
            let item = json!({"type":"compaction", "encrypted_content":"a".repeat(len),
                "vendor_field":"extra metadata".repeat(100)});
            assert_eq!(
                ResponsesContextEstimator
                    .estimate_compaction(&CompactionOutput::RemoteOpaque(item)),
                Some(tokens)
            );
        }
        assert_eq!(
            ResponsesContextEstimator.estimate_compaction(&CompactionOutput::RemoteOpaque(
                json!({"type":"other", "encrypted_content":"x"})
            )),
            None
        );
        assert_eq!(
            ResponsesContextEstimator.estimate_compaction(&CompactionOutput::RemoteWindow(vec![])),
            Some(0)
        );
    }
    #[test]
    fn complete_wire_window_estimation_keeps_unknown_media_and_items_unknown() {
        let estimate = |items| {
            ResponsesContextEstimator.estimate_compaction(&CompactionOutput::RemoteWindow(items))
        };
        assert!(
            estimate(vec![
                json!({"role":"developer","content":"rules"}),
                json!({"type":"tool_search_output","tools":[{"name":"read"}]})
            ])
            .unwrap()
                > 0
        );
        assert_eq!(
            estimate(vec![
                json!({"type":"compaction","encrypted_content":"a".repeat(12000)})
            ]),
            Some(2088)
        );
        assert_eq!(
            estimate(vec![
                json!({"type":"future_provider_item","payload":"unknown"})
            ]),
            None
        );
        for media in ["input_image", "input_audio"] {
            assert_eq!(
                estimate(vec![
                    json!({"type":"message","role":"user","content":[{"type":media,"data":"binary"}]})
                ]),
                None
            );
        }
    }
}
