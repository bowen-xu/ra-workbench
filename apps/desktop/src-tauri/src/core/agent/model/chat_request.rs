use base64::{Engine, engine::general_purpose::STANDARD};
use research_agent_core::model::{
    ContentPart, ModelCapabilities, ModelError, ModelMessage, ModelRequest,
};
use serde_json::{Value, json};

/// Wire parameter name is a provider capability, not a guessed model family.
#[derive(Clone, Copy)]
pub enum OutputTokenParameter {
    MaxTokens,
    MaxCompletionTokens,
}

pub(crate) fn encode(
    request: &ModelRequest,
    capabilities: ModelCapabilities,
    token_parameter: OutputTokenParameter,
) -> Result<Value, ModelError> {
    encode_with_developer_role(request, capabilities, token_parameter, false)
}

pub(super) fn encode_with_developer_role(
    request: &ModelRequest,
    capabilities: ModelCapabilities,
    token_parameter: OutputTokenParameter,
    developer_role: bool,
) -> Result<Value, ModelError> {
    request.validate(capabilities)?;
    if request.tools.iter().any(|tool| tool.namespace.is_some()) {
        return Err(ModelError::UnsupportedCapability);
    }
    // Responses can interleave assistant output with pending calls. Chat
    // requires the tool-result batch immediately after its assistant tool call;
    // do not silently reorder that history or merge distinct text items.
    let messages = canonical_messages(&request.messages, developer_role)?;
    let mut pending_results = 0;
    for item in &messages {
        match item["role"].as_str() {
            Some("assistant") => {
                if pending_results != 0 {
                    return Err(ModelError::UnsupportedCapability);
                }
                pending_results = item["tool_calls"].as_array().map_or(0, Vec::len);
            }
            Some("tool") => pending_results -= 1,
            _ => {}
        }
    }
    let mut body = json!({"model":request.model.model_id,"messages":messages,"stream":true,
        "n":1,"stream_options":{"include_usage":true}});
    body[match token_parameter {
        OutputTokenParameter::MaxTokens => "max_tokens",
        OutputTokenParameter::MaxCompletionTokens => "max_completion_tokens",
    }] = json!(request.max_output_tokens);
    if let Some(effort) = &request.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if !request.tools.is_empty() {
        body["tools"] = json!(
            request
                .tools
                .iter()
                .map(|tool| json!({"type":"function", "function":{
                    "name":tool.name,"description":tool.description,"parameters":tool.parameters
                }}))
                .collect::<Vec<_>>()
        );
        body["parallel_tool_calls"] = json!(request.parallel_tool_calls);
    }
    Ok(body)
}

fn message(message: &ModelMessage, developer_role: bool) -> Result<Value, ModelError> {
    Ok(match message {
        ModelMessage::System(text) => json!({"role":"system","content":text}),
        ModelMessage::Developer(text) if developer_role => {
            json!({"role":"developer","content":text})
        }
        ModelMessage::Developer(_) => return Err(ModelError::UnsupportedCapability),
        ModelMessage::User { content: parts, .. } => {
            json!({"role":"user","content":parts.iter().map(|part| match part {
            ContentPart::Text(text) => json!({"type":"text","text":text}),
            ContentPart::Image{media_type,bytes} => json!({"type":"image_url","image_url":{
                "url":format!("data:{media_type};base64,{}",STANDARD.encode(bytes))
            }}),
        }).collect::<Vec<_>>()})
        }
        ModelMessage::Assistant {
            content,
            tool_calls,
            ..
        } => {
            // Chat Completions accepts image input on user messages only.
            let mut text = String::new();
            for part in content {
                match part {
                    ContentPart::Text(value) => text.push_str(value),
                    ContentPart::Image { .. } => return Err(ModelError::UnsupportedCapability),
                }
            }
            let mut value = json!({"role":"assistant","content":if text.is_empty(){Value::Null}else{json!(text)}});
            if !tool_calls.is_empty() {
                if tool_calls.iter().any(|call| call.namespace.is_some()) {
                    return Err(ModelError::UnsupportedCapability);
                }
                value["tool_calls"] = json!(
                    tool_calls
                        .iter()
                        .map(|call| Ok(json!({
                        "id":call.call_id,"type":"function","function":{
                            "name":call.name,"arguments":call.arguments_json()?
                        }})))
                        .collect::<Result<Vec<_>, ModelError>>()?
                );
            }
            value
        }
        ModelMessage::ToolResult {
            call_id,
            content,
            is_error,
        } => {
            // The wire protocol has no is_error field. Preserve it explicitly
            // in the result text rather than presenting failure as success.
            let content = if *is_error {
                json!({"is_error":true,"content":content}).to_string()
            } else {
                content.clone()
            };
            json!({"role":"tool","tool_call_id":call_id,"content":content})
        }
        ModelMessage::CanonicalOutput(_) => return Err(ModelError::InvalidRequest),
    })
}

/// Chat's explicit compatibility projection can represent common messages and
/// adjacent call groups. Native-only material has no Chat representation and
/// must be retained in history while rejecting this request before HTTP.
fn canonical_messages(
    messages: &[ModelMessage],
    developer_role: bool,
) -> Result<Vec<Value>, ModelError> {
    use research_agent_core::model::ModelOutputItem;
    let mut result = Vec::new();
    let mut previous_call = false;
    for message in messages {
        let ModelMessage::CanonicalOutput(item) = message else {
            result.push(self::message(message, developer_role)?);
            previous_call = false;
            continue;
        };
        super::responses_chat_compatibility::validate(item)?;
        match &item.normalized {
            Some(ModelOutputItem::ReasoningSummary(_)) => {
                return Err(ModelError::UnsupportedCapability);
            }
            Some(ModelOutputItem::Message(message)) => {
                result.push(json!({"role":"assistant","content":if message.text.is_empty() {Value::Null} else {json!(message.text)}}));
                previous_call = false;
            }
            Some(ModelOutputItem::ToolCall(call)) => {
                let call = json!({"id":call.call_id,"type":"function","function":{
                    "name":call.name,"arguments":call.raw_arguments_json}});
                if previous_call {
                    result
                        .last_mut()
                        .and_then(|item| item["tool_calls"].as_array_mut())
                        .ok_or(ModelError::InvalidRequest)?
                        .push(call);
                } else {
                    result.push(json!({"role":"assistant","content":null,"tool_calls":[call]}));
                }
                previous_call = true;
            }
            None => return Err(ModelError::UnsupportedCapability),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        model::ToolCall,
        protocol::{AssistantMessagePhase, ModelIdentity},
    };

    #[test]
    fn namespaced_common_and_legacy_native_calls_are_rejected_before_chat_encoding() {
        use research_agent_core::model::{CompletedModelItem, ModelOutputItem, ToolCallIntent};
        for namespace in ["", "foreign::"] {
            let original = json!({"type":"function_call","id":"fc","call_id":"call","name":"read","arguments":"{","namespace":namespace});
            let native = super::super::responses_native::capture(
                &original,
                &super::super::responses_native::NativeScope::fixture(),
                None,
                None,
            )
            .unwrap();
            let mut legacy = native.clone();
            legacy
                .native
                .as_mut()
                .unwrap()
                .data
                .as_object_mut()
                .unwrap()
                .remove("namespace_from_common");
            legacy.native.as_mut().unwrap().data["item"]["namespace"] = json!(namespace);
            if let Some(ModelOutputItem::ToolCall(call)) = &mut legacy.normalized {
                call.namespace = None;
            }
            let common = CompletedModelItem {
                canonical_item_id: None,
                native: None,
                normalized: Some(ModelOutputItem::ToolCall(ToolCallIntent {
                    call_id: "call".into(),
                    name: "read".into(),
                    raw_arguments_json: "{".into(),
                    namespace: Some(namespace.into()),
                })),
            };
            for item in [common, native, legacy] {
                let request = ModelRequest {
                    request_id: uuid::Uuid::new_v4(),
                    captured_history: None,
                    model: ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "model".into(),
                        revision: None,
                    },
                    messages: vec![
                        ModelMessage::CanonicalOutput(Box::new(item)),
                        ModelMessage::ToolResult {
                            call_id: "call".into(),
                            content: "unsupported".into(),
                            is_error: true,
                        },
                    ],
                    tools: vec![],
                    max_output_tokens: 1,
                    parallel_tool_calls: false,
                    reasoning_effort: None,
                };
                assert_eq!(
                    encode(
                        &request,
                        ModelCapabilities {
                            vision: false,
                            tools: true,
                            parallel_tools: false,
                            reasoning_efforts: vec![]
                        },
                        OutputTokenParameter::MaxTokens
                    ),
                    Err(ModelError::UnsupportedCapability)
                );
            }
        }
    }

    #[test]
    fn interleaved_native_history_is_rejected_without_reordering_chat_messages() {
        let mut request = ModelRequest {
            request_id: uuid::Uuid::new_v4(),
            captured_history: None,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "model".into(),
                revision: None,
            },
            messages: vec![
                ModelMessage::Assistant {
                    phase: None,
                    content: vec![],
                    tool_calls: vec![ToolCall {
                        namespace: None,
                        call_id: "call-1".into(),
                        name: "read".into(),
                        arguments: json!({}),
                        raw_arguments_json: None,
                    }],
                },
                ModelMessage::Assistant {
                    phase: Some(AssistantMessagePhase::Commentary),
                    content: vec![ContentPart::Text("Reading".into())],
                    tool_calls: vec![],
                },
                ModelMessage::ToolResult {
                    call_id: "call-1".into(),
                    content: "source".into(),
                    is_error: false,
                },
            ],
            tools: vec![],
            max_output_tokens: 128,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let capabilities = ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        };
        assert_eq!(
            encode(
                &request,
                capabilities.clone(),
                OutputTokenParameter::MaxTokens
            ),
            Err(ModelError::UnsupportedCapability)
        );
        request.messages.remove(1);
        if let ModelMessage::Assistant { tool_calls, .. } = &mut request.messages[0] {
            tool_calls[0].namespace = Some("other".into());
        }
        assert_eq!(
            encode(
                &request,
                capabilities.clone(),
                OutputTokenParameter::MaxTokens
            ),
            Err(ModelError::UnsupportedCapability)
        );
        if let ModelMessage::Assistant { tool_calls, .. } = &mut request.messages[0] {
            tool_calls[0].namespace = None;
        }
        let body = encode(
            &request,
            capabilities.clone(),
            OutputTokenParameter::MaxTokens,
        )
        .unwrap();
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
        assert_eq!(body["messages"][1]["tool_call_id"], "call-1");
        request
            .tools
            .push(research_agent_core::model::ToolDefinition {
                namespace: Some("group".into()),
                name: "read".into(),
                description: "Read".into(),
                parameters: json!({"type":"object"}),
            });
        assert_eq!(
            encode(&request, capabilities, OutputTokenParameter::MaxTokens),
            Err(ModelError::UnsupportedCapability)
        );
    }

    #[test]
    fn chat_messages_do_not_send_responses_phase_metadata() {
        for phase in [
            None,
            Some(AssistantMessagePhase::Commentary),
            Some(AssistantMessagePhase::FinalAnswer),
        ] {
            let encoded = message(
                &ModelMessage::Assistant {
                    phase,
                    content: vec![ContentPart::Text("text".into())],
                    tool_calls: vec![],
                },
                false,
            )
            .unwrap();
            assert_eq!(encoded, json!({"role":"assistant","content":"text"}));
        }
    }

    #[test]
    fn canonical_raw_intents_encode_without_parsing_and_keep_adjacent_call_groups() {
        use research_agent_core::model::{CompletedModelItem, ModelOutputItem, ToolCallIntent};
        for raw in ["", "{", "[1]", "null", "17", "\"text\""] {
            let mut messages = vec![];
            for (id, args) in [("bad", raw), ("valid", "{}")] {
                messages.push(ModelMessage::CanonicalOutput(Box::new(
                    CompletedModelItem {
                        canonical_item_id: None,
                        native: None,
                        normalized: Some(ModelOutputItem::ToolCall(ToolCallIntent {
                            call_id: id.into(),
                            name: "read".into(),
                            raw_arguments_json: args.into(),
                            namespace: None,
                        })),
                    },
                )));
            }
            for id in ["bad", "valid"] {
                messages.push(ModelMessage::ToolResult {
                    call_id: id.into(),
                    content: "result".into(),
                    is_error: id == "bad",
                });
            }
            let request = ModelRequest {
                request_id: uuid::Uuid::new_v4(),
                captured_history: None,
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "model".into(),
                    revision: None,
                },
                messages,
                tools: vec![],
                max_output_tokens: 1,
                parallel_tool_calls: false,
                reasoning_effort: None,
            };
            let body = encode(
                &request,
                ModelCapabilities {
                    vision: false,
                    tools: true,
                    parallel_tools: false,
                    reasoning_efforts: vec![],
                },
                OutputTokenParameter::MaxTokens,
            )
            .unwrap();
            assert_eq!(body["messages"].as_array().unwrap().len(), 3);
            assert_eq!(
                body["messages"][0]["tool_calls"][0]["function"]["arguments"],
                raw
            );
            assert_eq!(body["messages"][0]["tool_calls"][1]["id"], "valid");
            assert_eq!(body["messages"][1]["tool_call_id"], "bad");
        }
    }
}
