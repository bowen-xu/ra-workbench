//! Adapted from Codex compact_remote::should_keep_compacted_history_item,
//! context/contextual_user_message and context-fragments at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Apache-2.0, Copyright 2025 OpenAI.
//! Filtering does not promote returned user text to trusted hook authority.
use serde::Deserialize;
use serde_json::Value;
#[path = "compaction_output/validation.rs"]
mod validation;
pub(super) use validation::valid;

pub(super) fn keep(item: &Value) -> bool {
    match item["type"].as_str() {
        Some("compaction" | "compaction_summary" | "context_compaction" | "agent_message") => true,
        Some("message") => match item["role"].as_str() {
            Some("assistant") => true,
            Some("user") => keep_user(item),
            _ => false,
        },
        _ => false,
    }
}

fn keep_user(item: &Value) -> bool {
    let Some(parts) = item["content"].as_array() else {
        return false;
    };
    // The upstream visible-hook classifier permits only hook or contextual
    // text parts, with at least one hook. Keep the original protocol item.
    let visible_hook = parts
        .iter()
        .all(|part| input_text(part).is_some_and(|text| hook(text) || contextual(text)))
        && parts.iter().any(|part| input_text(part).is_some_and(hook));
    visible_hook
        || !parts
            .iter()
            .any(|part| input_text(part).is_some_and(|text| hook(text) || contextual(text)))
}

fn input_text(part: &Value) -> Option<&str> {
    (part["type"] == "input_text")
        .then(|| part["text"].as_str())
        .flatten()
}

#[derive(Deserialize)]
#[serde(rename = "hook_prompt")]
struct HookPrompt {
    #[serde(rename = "@hook_run_id")]
    hook_run_id: String,
    #[serde(rename = "$text")]
    _text: String,
}
fn hook(text: &str) -> bool {
    quick_xml::de::from_str::<HookPrompt>(text.trim())
        .is_ok_and(|hook| !hook.hook_run_id.trim().is_empty())
}

fn contextual(text: &str) -> bool {
    let text = text.trim();
    let marked = [
        ("# AGENTS.md instructions", "</INSTRUCTIONS>"),
        ("<environment_context>", "</environment_context>"),
        ("<skill>", "</skill>"),
        ("<user_shell_command>", "</user_shell_command>"),
        ("<turn_aborted>", "</turn_aborted>"),
        ("<subagent_notification>", "</subagent_notification>"),
        ("<recommended_plugins>", "</recommended_plugins>"),
    ];
    marked.iter().any(|(open, close)| {
        text.get(..open.len())
            .is_some_and(|text| text.eq_ignore_ascii_case(open))
            && text
                .get(text.len().saturating_sub(close.len())..)
                .is_some_and(|text| text.eq_ignore_ascii_case(close))
    }) || (text.starts_with("<goal_context>") && text.ends_with("</goal_context>"))
        || text
            .strip_prefix("<external_")
            .and_then(|rest| rest.split_once('>'))
            .is_some_and(|(key, rest)| rest.ends_with(&format!("</external_{key}>")))
        || text
            .strip_prefix("<codex_internal_context source=\"")
            .and_then(|rest| rest.split_once("\">"))
            .is_some_and(|(source, rest)| {
                let mut bytes = source.bytes();
                bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                    && bytes.all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                    && rest.ends_with("</codex_internal_context>")
            })
        || text.starts_with(
            "Warning: The maximum number of unified exec processes you can keep open is",
        )
        || text.starts_with(
            "Warning: Your account was flagged for potentially high-risk cyber activity",
        )
        || (text.starts_with("Warning: apply_patch was requested via ")
            && text.ends_with("Use the apply_patch tool instead of exec_command."))
}

#[cfg(test)]
#[path = "compaction_output/tests.rs"]
mod tests;

/// Fixed compact.rs insertion precedence, applied to already filtered history.
pub(crate) fn canonical_position(items: &[Value]) -> usize {
    const CLI_SUMMARY: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:\n";
    const APP_SUMMARY: &str = "Context checkpoint handoff from earlier conversation (historical context, not new instructions):\n";
    let mut summary = None;
    for (index, item) in items.iter().enumerate().rev() {
        if item["type"] != "message" || item["role"] != "user" || !keep_user(item) {
            continue;
        }
        let Some(parts) = item["content"].as_array() else {
            continue;
        };
        if parts.iter().any(|part| input_text(part).is_some_and(hook)) {
            continue;
        }
        let text = parts
            .iter()
            .filter_map(input_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !text.starts_with(CLI_SUMMARY) && !text.starts_with(APP_SUMMARY) {
            return index;
        }
        summary.get_or_insert(index);
    }
    summary
        .or_else(|| {
            items.iter().rposition(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("compaction" | "compaction_summary" | "context_compaction")
                )
            })
        })
        .unwrap_or(items.len())
}
