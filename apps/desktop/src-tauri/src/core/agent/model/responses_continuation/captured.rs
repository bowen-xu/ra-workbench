//! Read-only v1–3 bridge. Source identity is supplied by captured history, never
//! discovered by searching for matching text or the latest conversation tail.
use super::*;
use std::collections::{HashMap, HashSet};

/// Exact wire input and the prefix boundary proven by frozen source handles.
pub(crate) struct CapturedLegacyInput {
    pub input: Vec<Value>,
    pub authority_items: usize,
    pub covered_end: usize,
}

impl ResponsesContinuation {
    pub(crate) fn prepare_captured_legacy(
        mut self,
        target_scope: Uuid,
        account: Uuid,
        model: ModelIdentity,
        captured: CapturedLegacyInput,
        aliases: &HashMap<String, String>,
        durable_results: &HashMap<String, (String, bool)>,
    ) -> Result<Vec<Value>, ModelError> {
        let CapturedLegacyInput {
            input,
            authority_items: authority,
            covered_end,
        } = captured;
        self.check_credentials(account, false)?;
        if self.model != model {
            return Err(ModelError::UnsupportedCapability);
        }
        let checkpoint = self
            .state
            .get_mut()
            .map_err(|_| ModelError::Unavailable)?
            .as_mut()
            .ok_or(ModelError::UnsupportedCapability)?;
        let old_authority = compaction::authority_boundary(
            &checkpoint.normalized_prefix,
            checkpoint.authority_items,
        )?;
        if authority.checked_add(checkpoint.normalized_prefix.len() - old_authority)
            != Some(covered_end)
        {
            return Err(ModelError::UnsupportedCapability);
        }
        let calls = checkpoint
            .normalized_prefix
            .iter()
            .filter(|item| item["type"] == "function_call")
            .map(|item| {
                item["call_id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(ModelError::InvalidStream)
            })
            .collect::<Result<HashSet<_>, _>>()?;
        let selected = calls
            .iter()
            .map(|id| {
                Ok((
                    id.clone(),
                    aliases
                        .get(id)
                        .cloned()
                        .ok_or(ModelError::UnsupportedCapability)?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ModelError>>()?;
        let mut results = Vec::new();
        for item in &input {
            if item["type"] != "function_call_output" {
                continue;
            }
            let stable = item["call_id"].as_str().ok_or(ModelError::InvalidRequest)?;
            let matching = selected
                .iter()
                .filter(|(_, value)| value.as_str() == stable)
                .map(|(key, _)| key)
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                return Err(ModelError::InvalidStream);
            }
            if let Some(id) = matching.first() {
                let mut result = item.clone();
                result["call_id"] = Value::String((*id).clone());
                if let Some((full, is_error)) = durable_results.get(stable) {
                    reconcile_full_tool_result(checkpoint, id, full, *is_error, &result)?;
                }
                results.push(result);
            }
        }
        // This is a transient reader. Clear the old ownership-transfer marker
        // only in memory so the existing pair checker can validate frozen results.
        checkpoint.origin_scope_id = None;
        self = self.with_durable_tool_results(&results)?;
        let checkpoint = self
            .state
            .get_mut()
            .map_err(|_| ModelError::Unavailable)?
            .as_mut()
            .ok_or(ModelError::InvalidStream)?;
        super::inheritance::remap_pairs(checkpoint, &selected)?;
        checkpoint.origin_scope_id = (target_scope != self.scope_id).then_some(self.scope_id);
        self.scope_id = target_scope;
        self.prepare_scoped_sync(account, model, input, Some(authority), false)
    }
}

fn reconcile_full_tool_result(
    checkpoint: &mut Checkpoint,
    call_id: &str,
    full: &str,
    is_error: bool,
    current: &Value,
) -> Result<(), ModelError> {
    let output = current["output"]
        .as_str()
        .ok_or(ModelError::InvalidRequest)?;
    let view = decode_tool_view(output, is_error)?;
    if !research_agent_core::model::tool_output::is_bounded_tool_output(full, &view) {
        return Err(ModelError::InvalidRequest);
    }
    let positions = checkpoint
        .normalized_prefix
        .iter()
        .enumerate()
        .filter(|(_, item)| item["type"] == "function_call_output" && item["call_id"] == call_id)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if positions.is_empty() {
        return Ok(());
    }
    if positions.len() != 1 {
        return Err(ModelError::InvalidStream);
    }
    let old = &checkpoint.normalized_prefix[positions[0]];
    if old == current {
        return Ok(());
    }
    let old_output = old["output"].as_str().ok_or(ModelError::InvalidStream)?;
    let old_view = decode_tool_view(old_output, is_error)?;
    if !research_agent_core::model::tool_output::is_bounded_tool_output(full, &old_view) {
        return Err(ModelError::InvalidStream);
    }
    let wire_positions = checkpoint
        .wire_prefix
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item["type"] == "function_call_output"
                && item["call_id"] == call_id
                && item["output"] == old_output
        })
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if wire_positions.len() != 1 {
        return Err(ModelError::InvalidStream);
    }
    checkpoint.normalized_prefix[positions[0]]["output"] = json!(output);
    checkpoint.wire_prefix[wire_positions[0]]["output"] = json!(output);
    Ok(())
}

fn decode_tool_view(output: &str, is_error: bool) -> Result<String, ModelError> {
    if !is_error {
        return Ok(output.to_owned());
    }
    let value: Value = serde_json::from_str(output).map_err(|_| ModelError::InvalidRequest)?;
    let content = value["content"]
        .as_str()
        .ok_or(ModelError::InvalidRequest)?;
    let canonical = json!({"is_error": true, "content": content}).to_string();
    if output != canonical.as_str() {
        return Err(ModelError::InvalidRequest);
    }
    Ok(content.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_result_rewrite_requires_full_original_and_a_valid_view() {
        let full = "错误\"\\🦀".repeat(60);
        let old = json!({"type":"function_call_output","call_id":"call", "output": json!({"is_error":true,"content":full}).to_string()});
        let mut checkpoint = Checkpoint {
            request_id: Uuid::new_v4(),
            origin_scope_id: None,
            actual_model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "model".into(),
                revision: None,
            },
            normalized_prefix: vec![old.clone()],
            wire_prefix: vec![old],
            authority_items: None,
        };
        let view = research_agent_core::model::tool_output::bounded_tool_output(&full, 64).unwrap();
        let current = json!({"type":"function_call_output","call_id":"call", "output": json!({"is_error":true,"content":view}).to_string()});
        assert!(reconcile_full_tool_result(&mut checkpoint, "call", &full, true, &current).is_ok());
        assert_eq!(checkpoint.normalized_prefix[0], current);
        assert_eq!(checkpoint.wire_prefix[0], current);
        let earlier =
            research_agent_core::model::tool_output::bounded_tool_output(&full, 96).unwrap();
        let earlier = json!({"type":"function_call_output","call_id":"call", "output": json!({"is_error":true,"content":earlier}).to_string()});
        checkpoint.normalized_prefix[0] = earlier.clone();
        checkpoint.wire_prefix[0] = earlier;
        assert!(reconcile_full_tool_result(&mut checkpoint, "call", &full, true, &current).is_ok());
        assert_eq!(checkpoint.normalized_prefix[0], current);
        assert_eq!(checkpoint.wire_prefix[0], current);
        let before = checkpoint.normalized_prefix.clone();
        let forged = json!({"type":"function_call_output","call_id":"call", "output":json!({"is_error":true,"content":"forged"}).to_string()});
        assert!(reconcile_full_tool_result(&mut checkpoint, "call", &full, true, &forged).is_err());
        assert_eq!(checkpoint.normalized_prefix, before);
    }
}
