//! Responses-specific reversible projection. Common bodies are held once in
//! canonical fields; native templates retain only metadata and byte bindings.
use research_agent_core::{
    model::{
        AssistantMessage, CompletedModelItem, ModelError, ModelMessage, ModelOutputItem,
        ModelRequest, ToolCallIntent,
    },
    protocol::{ModelIdentity, NativeItemData},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct NativeScope {
    pub model: ModelIdentity,
    pub account_scope: Uuid,
}
#[cfg(test)]
impl NativeScope {
    pub(crate) fn fixture() -> Self {
        Self {
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            account_scope: Uuid::from_u128(1),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Template {
    item: Value,
    #[serde(default)]
    text_bindings: Vec<TextBinding>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    namespace_from_common: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    summary_from_common: bool,
}
#[derive(Serialize, Deserialize)]
struct TextBinding {
    content_index: usize,
    field: String,
    start_byte: usize,
    len_byte: usize,
}

pub(super) fn capture(
    original: &Value,
    scope: &NativeScope,
    response_id: Option<String>,
    actual_model: Option<ModelIdentity>,
) -> Result<CompletedModelItem, ModelError> {
    if scope.account_scope.is_nil() {
        return Err(ModelError::Authentication);
    }
    let kind = string(original, "type")?;
    let item_id = string(original, "id")?.to_owned();
    let mut template = Template {
        item: original.clone(),
        text_bindings: vec![],
        namespace_from_common: false,
        summary_from_common: false,
    };
    let normalized = match kind {
        "message" => {
            if original["role"] != "assistant" {
                return Err(ModelError::InvalidStream);
            }
            let parts = template.item["content"]
                .as_array_mut()
                .ok_or(ModelError::InvalidStream)?;
            let mut text = String::new();
            for (content_index, part) in parts.iter_mut().enumerate() {
                let Some(field) = content_text_field(part)? else {
                    continue;
                };
                let body = part
                    .as_object_mut()
                    .ok_or(ModelError::InvalidStream)?
                    .remove(field)
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(ModelError::InvalidStream)?;
                template.text_bindings.push(TextBinding {
                    content_index,
                    field: field.into(),
                    start_byte: text.len(),
                    len_byte: body.len(),
                });
                text.push_str(&body);
            }
            Some(ModelOutputItem::Message(AssistantMessage {
                item_id: item_id.clone(),
                text,
                phase: super::responses_decoder::native_phase(original)?,
            }))
        }
        "function_call" => {
            let call_id = string(original, "call_id")?.to_owned();
            let name = string(original, "name")?.to_owned();
            let namespace = match original.get("namespace") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.clone()),
                _ => return Err(ModelError::InvalidStream),
            };
            let raw = original
                .get("arguments")
                .and_then(Value::as_str)
                .ok_or(ModelError::InvalidStream)?
                .to_owned();
            let fields = template
                .item
                .as_object_mut()
                .ok_or(ModelError::InvalidStream)?;
            fields.remove("arguments");
            fields.remove("name");
            fields.remove("call_id");
            if namespace.is_some() {
                fields.remove("namespace");
                template.namespace_from_common = true;
            }
            Some(ModelOutputItem::ToolCall(ToolCallIntent {
                call_id,
                name,
                raw_arguments_json: raw,
                namespace,
            }))
        }
        "reasoning" => Some(ModelOutputItem::ReasoningSummary(reasoning::capture(
            &mut template,
            item_id.clone(),
        )?)),
        _ => return Err(ModelError::UnsupportedResponseType),
    };
    let mut native = NativeItemData::new(
        "responses".into(),
        scope.model.clone(),
        kind.into(),
        serde_json::to_value(template).map_err(|_| ModelError::InvalidStream)?,
    );
    native.account_scope = Some(scope.account_scope);
    native.actual_model = actual_model;
    native.response_id = response_id;
    native.item_id = Some(item_id);
    let result = CompletedModelItem {
        canonical_item_id: None,
        normalized,
        native: Some(Box::new(native)),
    };
    result.validate()?;
    // Validation also proves the binding covers exactly the canonical body.
    if render(&result)? != *original {
        return Err(ModelError::InvalidStream);
    }
    Ok(result)
}

pub(crate) fn render(item: &CompletedModelItem) -> Result<Value, ModelError> {
    item.validate()?;
    let native = item
        .native
        .as_ref()
        .ok_or(ModelError::UnsupportedCapability)?;
    if native.api != "responses" {
        return Err(ModelError::UnsupportedCapability);
    }
    let mut template: Template =
        serde_json::from_value(native.data.clone()).map_err(|_| ModelError::InvalidStream)?;
    if template.item["type"].as_str() != Some(native.item_type.as_str())
        || template.item["id"].as_str() != native.item_id.as_deref()
    {
        return Err(ModelError::InvalidStream);
    }
    match (&item.normalized, native.item_type.as_str()) {
        (Some(ModelOutputItem::Message(message)), "message") => {
            if template.summary_from_common
                || template.namespace_from_common
                || template.item["role"] != "assistant"
                || super::responses_decoder::native_phase(&template.item)? != message.phase
            {
                return Err(ModelError::InvalidStream);
            }
            let parts = template.item["content"]
                .as_array_mut()
                .ok_or(ModelError::InvalidStream)?;
            let mut cursor = 0;
            let mut bindings = template.text_bindings.into_iter();
            for (content_index, part) in parts.iter_mut().enumerate() {
                let Some(expected) = content_text_field(part)? else {
                    continue;
                };
                let binding = bindings.next().ok_or(ModelError::InvalidStream)?;
                let end = binding
                    .start_byte
                    .checked_add(binding.len_byte)
                    .ok_or(ModelError::InvalidStream)?;
                if binding.content_index != content_index
                    || binding.field != expected
                    || binding.start_byte != cursor
                    || !part[expected].is_null()
                {
                    return Err(ModelError::InvalidStream);
                }
                let text = message
                    .text
                    .get(cursor..end)
                    .ok_or(ModelError::InvalidStream)?;
                part[expected] = Value::String(text.into());
                cursor = end;
            }
            if bindings.next().is_some() {
                return Err(ModelError::InvalidStream);
            }
            if cursor != message.text.len() {
                return Err(ModelError::InvalidStream);
            }
        }
        (Some(ModelOutputItem::ToolCall(call)), "function_call") => {
            if template.summary_from_common
                || !template.text_bindings.is_empty()
                || !template.item["arguments"].is_null()
                || !template.item["name"].is_null()
                || !template.item["call_id"].is_null()
            {
                return Err(ModelError::InvalidStream);
            }
            template.item["arguments"] = Value::String(call.raw_arguments_json.clone());
            template.item["name"] = Value::String(call.name.clone());
            template.item["call_id"] = Value::String(call.call_id.clone());
            if template.namespace_from_common {
                if template.item.get("namespace").is_some() {
                    return Err(ModelError::InvalidStream);
                }
                template.item["namespace"] =
                    Value::String(call.namespace.clone().ok_or(ModelError::InvalidStream)?);
            } else if call.namespace.is_some() {
                return Err(ModelError::InvalidStream);
            }
        }
        (Some(ModelOutputItem::ReasoningSummary(summary)), "reasoning") => {
            reasoning::render(&mut template, summary)?
        }
        (None, "reasoning")
            if template.text_bindings.is_empty()
                && !template.namespace_from_common
                && !template.summary_from_common => {}
        _ => return Err(ModelError::InvalidStream),
    }
    Ok(template.item)
}

/// Older templates retained namespace inside native JSON. Chat must reject
/// those too, rather than silently executing/replaying the same bare name.
pub(super) fn has_tool_namespace(item: &CompletedModelItem) -> bool {
    if !matches!(&item.normalized, Some(ModelOutputItem::ToolCall(_))) {
        return false;
    }
    matches!(&item.normalized,Some(ModelOutputItem::ToolCall(call)) if call.namespace.is_some())
        || item.native.as_ref().is_some_and(|native| {
            native.api == "responses"
                && (native.data["namespace_from_common"] == true
                    || !native.data["item"]["namespace"].is_null())
        })
}

pub(crate) fn validate_scope(request: &ModelRequest, account: Uuid) -> Result<(), ModelError> {
    validate_scope_inner(request, account, false)
}

/// Codex replays the original Responses items across models within the same
/// ChatGPT account. Other transports and legacy compaction stay exact-model.
pub(crate) fn validate_chatgpt_scope(
    request: &ModelRequest,
    account: Uuid,
) -> Result<(), ModelError> {
    validate_scope_inner(request, account, true)
}

fn validate_scope_inner(
    request: &ModelRequest,
    account: Uuid,
    chatgpt: bool,
) -> Result<(), ModelError> {
    for message in &request.messages {
        if let ModelMessage::CanonicalOutput(item) = message
            && let Some(native) = &item.native
        {
            native.validate().map_err(|_| ModelError::InvalidStream)?;
            let same_chatgpt_provider = chatgpt
                && native.model.provider_id == "openai"
                && request.model.provider_id == "openai";
            if native.api != "responses"
                || (native.model != request.model && !same_chatgpt_provider)
            {
                return Err(ModelError::UnsupportedCapability);
            }
            if native.account_scope != Some(account) {
                return Err(ModelError::Authentication);
            }
        }
    }
    Ok(())
}

/// The pinned Codex ContentItem accepts text plus image/audio input parts in a
/// message. Media stays native-only; the common message contains readable text.
pub(super) fn content_text_field(part: &Value) -> Result<Option<&'static str>, ModelError> {
    match string(part, "type")? {
        "input_text" | "output_text" => Ok(Some("text")),
        "refusal" => Ok(Some("refusal")),
        "input_image" if part["image_url"].is_string() => Ok(None),
        "input_audio" if part["audio_url"].is_string() => Ok(None),
        "input_image" | "input_audio" => Err(ModelError::InvalidStream),
        _ => Err(ModelError::UnsupportedResponseType),
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ModelError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ModelError::InvalidStream)
}

pub(crate) fn capture_request(
    request: &ModelRequest,
    input_len: usize,
    text_compaction: bool,
    sampling: bool,
) -> Result<super::responses_metadata::CapturedResponsesInput, ModelError> {
    use super::responses_metadata::{CapturedInputItem, CapturedResponsesInput};
    let mut result = CapturedResponsesInput {
        request_id: request.request_id,
        sampling,
        authority_items: request
            .messages
            .iter()
            .take_while(|message| matches!(message, ModelMessage::System(_)))
            .count(),
        text_compaction,
        history: request.captured_history.clone(),
        items: vec![],
        users: vec![],
    };
    let mut input_index = 0;
    for message in &request.messages {
        let slots = match message {
            ModelMessage::CanonicalOutput(item) => {
                let slots = usize::from(
                    item.native.is_some()
                        || !matches!(&item.normalized,Some(ModelOutputItem::Message(message)) if message.text.is_empty()),
                );
                if let Some(item_id) = item.canonical_item_id
                    && slots != 0
                {
                    result.items.push(CapturedInputItem {
                        item_id,
                        input_index,
                        native: item.native.is_some(),
                    });
                }
                slots
            }
            ModelMessage::User { source_item_id, .. } => {
                if let Some(item_id) = source_item_id {
                    result.users.push(CapturedInputItem {
                        item_id: *item_id,
                        input_index,
                        native: false,
                    });
                }
                1
            }
            ModelMessage::Assistant {
                content,
                tool_calls,
                ..
            } => usize::from(!content.is_empty()) + tool_calls.len(),
            _ => 1,
        };
        input_index = input_index
            .checked_add(slots)
            .ok_or(ModelError::BudgetExceeded)?;
    }
    if input_index != input_len {
        return Err(ModelError::InvalidRequest);
    }
    Ok(result)
}

#[path = "responses_native/reasoning.rs"]
mod reasoning;
#[cfg(test)]
#[path = "responses_native/tests.rs"]
mod tests;
