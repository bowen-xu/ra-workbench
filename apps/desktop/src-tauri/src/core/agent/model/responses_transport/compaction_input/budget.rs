//! Tail-only request rewrite migrated from Codex compact_remote.rs, fixed
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
use super::*;
use research_agent_core::context::{
    accounting::ContextAccounting, compaction_strategy::CompactionOutput,
};

const TRUNCATED: &str = "Output exceeded the available model context and was truncated";

pub(in crate::agent::model::responses_transport) fn trim_body(
    body: &mut Value,
    request: &ModelRequest,
    accounting: Option<&ContextAccounting>,
) -> Result<(), ModelError> {
    let Some(accounting) = accounting else {
        return Ok(());
    };
    if accounting.model != request.model {
        return Err(ModelError::InvalidRequest);
    }
    let Some(budget) = accounting.budget else {
        return Ok(());
    };
    budget.limits().map_err(|_| ModelError::InvalidRequest)?;
    let effective = (u128::from(budget.context_window_tokens)
        * u128::from(budget.effective_percent)
        / 100) as u64;
    let Some(tool_tokens) = accounting.estimator.estimate(&[], &request.tools) else {
        return Ok(());
    };
    let input = body["input"]
        .as_array_mut()
        .ok_or(ModelError::InvalidRequest)?;
    // V2's client-added trigger is outside the active history to rewrite.
    let end = input.len()
        - usize::from(
            input
                .last()
                .is_some_and(|item| item["type"] == "compaction_trigger"),
        );
    for index in (0..end).rev() {
        let Some(tokens) = accounting
            .estimator
            .estimate_compaction(&CompactionOutput::RemoteWindow(input[..end].to_vec()))
        else {
            break;
        };
        let total = tokens
            .checked_add(tool_tokens)
            .ok_or(ModelError::BudgetExceeded)?;
        if total <= effective {
            break;
        }
        if !rewrite(&mut input[index]) {
            break;
        }
    }
    Ok(())
}

fn rewrite(item: &mut Value) -> bool {
    match item["type"].as_str() {
        Some("function_call_output" | "custom_tool_call_output") => {
            let output = &mut item["output"];
            if !(output.is_string() || output.is_array()) {
                return false;
            }
            // Keep this application's normalized error envelope, if present.
            let error = output
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .filter(|value| {
                    value.as_object().is_some_and(|object| object.len() == 2)
                        && value["is_error"] == true
                        && value["content"].is_string()
                });
            *output = if let Some(mut error) = error {
                error["content"] = json!(TRUNCATED);
                json!(error.to_string())
            } else {
                json!(TRUNCATED)
            };
            true
        }
        Some("tool_search_output") if item["tools"].is_array() => {
            item["tools"] = json!([]);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "budget/tests.rs"]
mod tests;
