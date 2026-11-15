//! Explicit compatibility with the subset represented by our Chat encoder.
//! Provider facts stay in history. Unsupported material is never replaced by
//! the readable text subset just because it has a normalized message too.
use research_agent_core::model::{CompletedModelItem, ModelError, ModelOutputItem};
use serde_json::Value;

const TEMPLATE_FIELDS: &[&str] = &["item", "text_bindings", "namespace_from_common"];
const BINDING_FIELDS: &[&str] = &["content_index", "field", "start_byte", "len_byte"];
const MESSAGE_FIELDS: &[&str] = &["type", "id", "role", "phase", "status", "content"];
const TEXT_FIELDS: &[&str] = &["type", "text", "annotations", "logprobs"];
const CALL_FIELDS: &[&str] = &[
    "type",
    "id",
    "status",
    "call_id",
    "name",
    "arguments",
    "namespace",
];

/// Validate before constructing a Chat HTTP body. This is a capability boundary
/// of this adapter, not a restriction on what a Responses decoder may retain.
pub(super) fn validate(item: &CompletedModelItem) -> Result<(), ModelError> {
    item.validate()?;
    if matches!(item.normalized, Some(ModelOutputItem::ReasoningSummary(_))) {
        return Err(ModelError::UnsupportedCapability);
    }
    let Some(native) = &item.native else {
        return reject_namespace(item);
    };
    if item.normalized.is_none()
        || native.api != "responses"
        || !matches!(native.item_type.as_str(), "message" | "function_call")
    {
        return Err(ModelError::UnsupportedCapability);
    }

    // Reuse the authoritative codec for roles, phases and exact UTF-8 bindings;
    // malformed native facts must not fall back to their normalized text.
    let original = super::responses_native::render(item)?;
    reject_namespace(item)?;
    if !native.extra.is_empty() || !known_template(&native.data, &native.item_type) {
        return Err(ModelError::UnsupportedCapability);
    }
    let compatible = match &item.normalized {
        Some(ModelOutputItem::Message(message)) => plain_message(&original, &message.text),
        Some(ModelOutputItem::ToolCall(_)) => {
            only_fields(&original, CALL_FIELDS)
                && record_metadata(&original)
                && original["namespace"].is_null()
        }
        Some(ModelOutputItem::ReasoningSummary(_)) | None => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(ModelError::UnsupportedCapability)
    }
}

fn known_template(data: &Value, kind: &str) -> bool {
    only_fields(data, TEMPLATE_FIELDS)
        && match data.get("text_bindings") {
            None | Some(Value::Null) => true,
            Some(Value::Array(bindings)) => {
                (kind == "message" || bindings.is_empty())
                    && bindings
                        .iter()
                        .all(|value| only_fields(value, BINDING_FIELDS))
            }
            _ => false,
        }
        && match data.get("namespace_from_common") {
            None | Some(Value::Bool(false)) => true,
            Some(Value::Bool(true)) => kind == "function_call",
            _ => false,
        }
}

fn reject_namespace(item: &CompletedModelItem) -> Result<(), ModelError> {
    if super::responses_native::has_tool_namespace(item) {
        Err(ModelError::UnsupportedCapability)
    } else {
        Ok(())
    }
}

fn plain_message(item: &Value, common: &str) -> bool {
    let mut offset: usize = 0;
    only_fields(item, MESSAGE_FIELDS)
        && record_metadata(item)
        && item["content"].as_array().is_some_and(|parts| {
            parts.iter().all(|part| {
                let Some(text) = part["text"].as_str() else {
                    return false;
                };
                let Some(end) = offset.checked_add(text.len()) else {
                    return false;
                };
                let same_text = common.get(offset..end) == Some(text);
                offset = end;
                only_fields(part, TEXT_FIELDS)
                    && matches!(part["type"].as_str(), Some("input_text" | "output_text"))
                    && empty_observation(part.get("annotations"))
                    && empty_observation(part.get("logprobs"))
                    && same_text
            })
        })
        && offset == common.len()
}

fn record_metadata(item: &Value) -> bool {
    // IDs, output status and the already-validated phase describe the accepted
    // record. They do not add Chat message content. Structured/unknown values
    // are not assumed to be harmless metadata.
    ["id", "status"]
        .iter()
        .all(|field| item[*field].is_null() || item[*field].is_string())
}

fn empty_observation(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.is_null() || value.as_array().is_some_and(Vec::is_empty))
}

fn only_fields(value: &Value, fields: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| fields.contains(&key.as_str())))
}

#[cfg(test)]
#[path = "responses_chat_compatibility/tests.rs"]
mod tests;
