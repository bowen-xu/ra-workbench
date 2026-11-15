//! Responses request mapping, adapted from Codex's fixed ResponsesApiRequest
//! contract. Application history stays provider-neutral; no Codex thread IDs.
// Copyright 2025 OpenAI. Adapted under Apache-2.0; local differences recorded in
// docs/architecture/shared-agent-codex-migration-map.md.
use base64::{Engine, engine::general_purpose::STANDARD};
use research_agent_core::model::{
    ContentPart, ModelCapabilities, ModelError, ModelMessage, ModelRequest,
};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub enum ResponsesEndpointKind {
    Platform,
    ChatGpt,
}

/// Fixed Codex V2 trigger, distinct from a user /compact prompt and from the
/// legacy /responses/compact endpoint. Capability is chosen by the native host.
pub fn encode_responses_compaction_request(
    request: &ModelRequest,
    capabilities: ModelCapabilities,
    endpoint: ResponsesEndpointKind,
) -> Result<Value, ModelError> {
    let mut body = encode_responses_request(request, capabilities, endpoint)?;
    body["input"]
        .as_array_mut()
        .ok_or(ModelError::InvalidRequest)?
        .push(json!({"type":"compaction_trigger"}));
    Ok(body)
}

/// Encodes the frozen canonical history. Native templates restore exact provider
/// fields from the same common items; legacy storage expansion stays in the port.
pub fn encode_responses_request(
    request: &ModelRequest,
    capabilities: ModelCapabilities,
    endpoint: ResponsesEndpointKind,
) -> Result<Value, ModelError> {
    request.validate(capabilities)?;
    let mut input = Vec::new();
    for message in &request.messages {
        match message {
            ModelMessage::CanonicalOutput(item) => {
                if item.native.is_some() { input.push(super::responses_native::render(item)?); }
                else if let Some(output)=&item.normalized {
                    if let Some(value)=encode_common_output(output)? {input.push(value);}
                } else { return Err(ModelError::InvalidRequest); }
            }
            ModelMessage::System(text) | ModelMessage::Developer(text) => input.push(json!({"role":"developer", "content": text})),
            ModelMessage::User { content:parts, .. } => input.push(json!({"role":"user", "content": parts.iter().map(|part| {
                match part {
                    ContentPart::Text(text) => json!({"type":"input_text", "text":text}),
                    ContentPart::Image {media_type, bytes} => json!({"type":"input_image",
                        "image_url":format!("data:{media_type};base64,{}", STANDARD.encode(bytes))}),
                }
            }).collect::<Vec<_>>()})),
            ModelMessage::Assistant {content, tool_calls, phase} => {
                let mut parts = Vec::new();
                for part in content {
                    match part {
                        ContentPart::Text(text) => parts.push(json!({"type":"output_text", "text":text})),
                        ContentPart::Image {..} => return Err(ModelError::UnsupportedCapability),
                    }
                }
                if !parts.is_empty() {
                    let mut message = json!({"type":"message", "role":"assistant", "content":parts});
                    if let Some(phase) = phase {
                        message["phase"] = json!(match phase {
                            research_agent_core::protocol::AssistantMessagePhase::Commentary => "commentary",
                            research_agent_core::protocol::AssistantMessagePhase::FinalAnswer => "final_answer",
                            // Reasoning is a local activity role, not a Responses message phase.
                            research_agent_core::protocol::AssistantMessagePhase::Reasoning => return Err(ModelError::InvalidRequest),
                        });
                    }
                    input.push(message);
                }
                for call in tool_calls {
                    let mut item = json!({"type":"function_call", "call_id":call.call_id,
                        "name":call.name, "arguments":call.arguments_json()?});
                    if let Some(namespace) = &call.namespace {
                        item["namespace"] = json!(namespace);
                    }
                    input.push(item);
                }
            }
            ModelMessage::ToolResult {call_id, content, is_error} => {
                let output = if *is_error {json!({"is_error":true, "content":content}).to_string()}
                    else {content.clone()};
                input.push(json!({"type":"function_call_output", "call_id":call_id, "output":output}));
            }
        }
    }
    let mut body = json!({"model":request.model.model_id, "input":input,
        "stream":true, "store":false, "parallel_tool_calls":request.parallel_tool_calls,
        "tools":encode_tools(&request.tools),
        "tool_choice":"auto", "include":["reasoning.encrypted_content"]});
    if let Some(effort) = &request.reasoning_effort {
        body["reasoning"] = json!({"effort":effort});
    }
    match endpoint {
        ResponsesEndpointKind::Platform => {
            body["max_output_tokens"] = json!(request.max_output_tokens)
        }
        // The pinned Codex request has no max_output_tokens field. Native output
        // budgets remain required; do not promise a server-side token cap here.
        ResponsesEndpointKind::ChatGpt => body["instructions"] = json!(""),
    }
    Ok(body)
}

/// Codex coalesces namespace specifications at their first catalog position.
fn encode_tools(tools: &[research_agent_core::model::ToolDefinition]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    let mut namespaces = std::collections::HashMap::new();
    for tool in tools {
        let function = json!({"type":"function", "name":tool.name,
            "description":tool.description, "parameters":tool.parameters, "strict":false});
        if let Some(namespace) = &tool.namespace {
            let index = *namespaces.entry(namespace).or_insert_with(|| {
                let index = result.len();
                result.push(json!({"type":"namespace", "name":namespace,
                    "description":format!("Tools in the {namespace} namespace."), "tools":[]}));
                index
            });
            result[index]["tools"]
                .as_array_mut()
                .expect("namespace tools is an array")
                .push(function);
        } else {
            result.push(function);
        }
    }
    for index in namespaces.values() {
        result[*index]["tools"]
            .as_array_mut()
            .expect("namespace tools is an array")
            .sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    }
    result
}

fn encode_common_output(
    item: &research_agent_core::model::ModelOutputItem,
) -> Result<Option<Value>, ModelError> {
    use research_agent_core::{model::ModelOutputItem, protocol::AssistantMessagePhase};
    match item {
        ModelOutputItem::ReasoningSummary(summary) => {
            let mut parts = vec![];
            for (index, part) in summary.parts.iter().enumerate() {
                if part.index.as_str() != index.to_string() {
                    return Err(ModelError::UnsupportedCapability);
                }
                let end = part
                    .start_byte
                    .checked_add(part.len_byte)
                    .ok_or(ModelError::InvalidRequest)?;
                parts.push(json!({"type":"summary_text","text":summary.text.get(part.start_byte..end).ok_or(ModelError::InvalidRequest)?}));
            }
            Ok(Some(
                json!({"type":"reasoning","id":summary.item_id,"summary":parts}),
            ))
        }
        ModelOutputItem::Message(message) => {
            if message.text.is_empty() {
                return Ok(None);
            }
            let mut value = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":message.text}]});
            if let Some(phase) = message.phase {
                value["phase"] = json!(match phase {
                    AssistantMessagePhase::Commentary => "commentary",
                    AssistantMessagePhase::FinalAnswer => "final_answer",
                    AssistantMessagePhase::Reasoning => return Err(ModelError::InvalidRequest),
                });
            }
            Ok(Some(value))
        }
        ModelOutputItem::ToolCall(call) => {
            let mut value = json!({"type":"function_call","call_id":call.call_id,"name":call.name,"arguments":call.raw_arguments_json});
            if let Some(namespace) = &call.namespace {
                value["namespace"] = json!(namespace);
            }
            Ok(Some(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        model::{ToolCall, ToolDefinition},
        protocol::ModelIdentity,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    fn request() -> ModelRequest {
        ModelRequest {
            request_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "openai".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            messages: vec![
                ModelMessage::System("Research rules".into()),
                ModelMessage::User {
                    content: vec![
                        ContentPart::Text("Read this".into()),
                        ContentPart::Image {
                            media_type: "image/png".into(),
                            bytes: Arc::from([1, 2]),
                        },
                    ],
                    source_item_id: None,
                },
                ModelMessage::Assistant {
                    phase: None,
                    content: vec![],
                    tool_calls: vec![ToolCall {
                        namespace: None,
                        call_id: "call-1".into(),
                        name: "read".into(),
                        arguments: json!({"path":"a.tex"}),
                        raw_arguments_json: None,
                    }],
                },
                ModelMessage::ToolResult {
                    call_id: "call-1".into(),
                    content: "missing".into(),
                    is_error: true,
                },
            ],
            tools: vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "Read source".into(),
                parameters: json!({"type":"object"}),
            }],
            max_output_tokens: 4000,
            parallel_tool_calls: false,
            reasoning_effort: Some("medium".into()),
            captured_history: None,
        }
    }
    fn capabilities() -> ModelCapabilities {
        ModelCapabilities {
            vision: true,
            tools: true,
            parallel_tools: true,
            reasoning_efforts: vec!["medium".into()],
        }
    }

    #[test]
    fn parsed_call_namespace_is_preserved_on_responses_history() {
        let mut request = request();
        let ModelMessage::Assistant { tool_calls, .. } = &mut request.messages[2] else {
            panic!("expected assistant");
        };
        tool_calls[0].namespace = Some("tools.".into());
        let encoded =
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform)
                .unwrap();
        let call = encoded["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        assert_eq!(call["namespace"], "tools.");
        assert_eq!(call["name"], "read");
    }

    #[test]
    fn namespace_catalog_matches_codex_grouping_and_keeps_plain_identity() {
        let mut request = request();
        let mut qualified = request.tools[0].clone();
        qualified.namespace = Some("tools.".into());
        qualified.name = "write".into();
        request.tools.push(qualified.clone());
        qualified.name = "read".into();
        request.tools.push(qualified);
        let encoded =
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform)
                .unwrap();
        assert_eq!(encoded["tools"][0]["type"], "function");
        assert_eq!(encoded["tools"][0]["name"], "read");
        assert_eq!(encoded["tools"][1]["type"], "namespace");
        assert_eq!(encoded["tools"][1]["name"], "tools.");
        assert_eq!(
            encoded["tools"][1]["description"],
            "Tools in the tools. namespace."
        );
        assert_eq!(encoded["tools"][1]["tools"][0]["name"], "read");
        assert_eq!(encoded["tools"][1]["tools"][1]["name"], "write");
    }

    #[test]
    fn namespaces_keep_first_catalog_position_and_sort_only_their_members() {
        let tool = |namespace: Option<&str>, name: &str| ToolDefinition {
            namespace: namespace.map(str::to_owned),
            name: name.into(),
            description: name.into(),
            parameters: json!({"type":"object"}),
        };
        let catalog = encode_tools(&[
            tool(Some("alpha"), "z"),
            tool(None, "plain"),
            tool(Some("beta"), "z"),
            tool(Some("alpha"), "a"),
            tool(Some("beta"), "a"),
        ]);
        assert_eq!(
            catalog
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["alpha", "plain", "beta"]
        );
        for index in [0, 2] {
            let names = catalog[index]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(names, ["a", "z"]);
        }
    }

    #[test]
    fn assistant_message_phases_are_replayed_without_coalescing_items() {
        use research_agent_core::protocol::AssistantMessagePhase;
        let mut request = request();
        request.messages = vec![
            ModelMessage::User {
                content: vec![ContentPart::Text("inspect".into())],
                source_item_id: None,
            },
            ModelMessage::Assistant {
                phase: Some(AssistantMessagePhase::Commentary),
                content: vec![ContentPart::Text("Checking the source.".into())],
                tool_calls: vec![],
            },
            ModelMessage::Assistant {
                phase: Some(AssistantMessagePhase::FinalAnswer),
                content: vec![ContentPart::Text("The source confirms it.".into())],
                tool_calls: vec![],
            },
            ModelMessage::Assistant {
                phase: None,
                content: vec![ContentPart::Text("Legacy text.".into())],
                tool_calls: vec![],
            },
        ];
        for endpoint in [
            ResponsesEndpointKind::Platform,
            ResponsesEndpointKind::ChatGpt,
        ] {
            let body = encode_responses_request(&request, capabilities(), endpoint).unwrap();
            assert_eq!(body["input"].as_array().unwrap().len(), 4);
            assert_eq!(body["input"][1]["phase"], "commentary");
            assert_eq!(body["input"][2]["phase"], "final_answer");
            assert!(body["input"][3].get("phase").is_none());
        }
    }

    #[test]
    fn commentary_after_a_tool_call_keeps_its_place_before_the_result() {
        use research_agent_core::protocol::AssistantMessagePhase;
        let mut request = request();
        request.messages.insert(
            3,
            ModelMessage::Assistant {
                phase: Some(AssistantMessagePhase::Commentary),
                content: vec![ContentPart::Text("Reading the requested source.".into())],
                tool_calls: vec![],
            },
        );
        let body =
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform)
                .unwrap();
        assert_eq!(body["input"][2]["type"], "function_call");
        assert_eq!(body["input"][3]["phase"], "commentary");
        assert_eq!(body["input"][4]["type"], "function_call_output");
        assert_eq!(body["input"][4]["call_id"], "call-1");
    }

    #[test]
    fn developer_fragments_preserve_role_and_order_without_entering_user_content() {
        let mut request = request();
        request
            .messages
            .insert(1, ModelMessage::Developer("trusted guidance".into()));
        for endpoint in [
            ResponsesEndpointKind::Platform,
            ResponsesEndpointKind::ChatGpt,
        ] {
            let body = encode_responses_request(&request, capabilities(), endpoint).unwrap();
            assert_eq!(
                body["input"][1],
                json!({"role":"developer", "content":"trusted guidance"})
            );
            assert_eq!(body["input"][2]["role"], "user");
        }
    }

    #[test]
    fn maps_ordered_multimodal_history_and_explicit_profiles() {
        let request = request();
        let platform =
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform)
                .unwrap();
        assert_eq!(platform["input"][0]["role"], "developer");
        assert_eq!(
            platform["input"][1]["content"][1]["image_url"],
            "data:image/png;base64,AQI="
        );
        assert_eq!(platform["input"][2]["call_id"], "call-1");
        assert_eq!(platform["input"][3]["call_id"], "call-1");
        let output: Value =
            serde_json::from_str(platform["input"][3]["output"].as_str().unwrap()).unwrap();
        assert_eq!(output, json!({"is_error":true,"content":"missing"}));
        assert_eq!(platform["max_output_tokens"], 4000);
        assert_eq!(platform["store"], false);
        assert_eq!(platform["reasoning"]["effort"], "medium");
        assert!(platform.get("previous_response_id").is_none());
        let chatgpt =
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::ChatGpt)
                .unwrap();
        assert!(chatgpt.get("max_output_tokens").is_none());
        assert_eq!(chatgpt["instructions"], "");
        assert_eq!(chatgpt["reasoning"]["effort"], "medium");
    }

    #[test]
    fn refuses_lossy_or_orphaned_history() {
        let mut request = request();
        request.messages.pop();
        assert!(matches!(
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform),
            Err(ModelError::InvalidRequest)
        ));
        let mut request = self::request();
        request.messages.push(ModelMessage::Assistant {
            phase: None,
            content: vec![ContentPart::Image {
                media_type: "image/png".into(),
                bytes: Arc::from([1]),
            }],
            tool_calls: vec![],
        });
        assert!(matches!(
            encode_responses_request(&request, capabilities(), ResponsesEndpointKind::Platform),
            Err(ModelError::UnsupportedCapability)
        ));
    }
}
