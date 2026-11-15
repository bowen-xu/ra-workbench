//! Explicit provider wire behavior, selected by the native capability profile.
use research_agent_core::model::{ModelError, ModelRequest};
use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
pub enum ChatRequestPolicy {
    #[default]
    ProviderDefault,
    /// Request a JSON object through the provider-neutral Chat Completions adapter.
    JsonObject,
    /// DeepSeek thinking tool calls require opaque reasoning replay, which the
    /// shared adapter does not yet implement. This mode is explicitly disabled,
    /// never inferred from a model name or used to downgrade requested effort.
    DeepSeekNonThinking,
}
impl ChatRequestPolicy {
    pub(super) fn apply(self, request: &ModelRequest, body: &mut Value) -> Result<(), ModelError> {
        match self {
            Self::ProviderDefault => Ok(()),
            Self::JsonObject => {
                body["response_format"] = json!({"type": "json_object"});
                Ok(())
            }
            Self::DeepSeekNonThinking => {
                if request.model.provider_id != "deepseek" || request.reasoning_effort.is_some() {
                    return Err(ModelError::UnsupportedCapability);
                }
                body["thinking"] = json!({"type": "disabled"});
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{model::*, protocol::ModelIdentity};
    fn request() -> ModelRequest {
        ModelRequest {
            captured_history: None,
            request_id: uuid::Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "deepseek".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            messages: vec![ModelMessage::User {
                content: vec![ContentPart::Text("Explain".into())],
                source_item_id: None,
            }],
            tools: vec![],
            parallel_tool_calls: false,
            max_output_tokens: 100,
        }
    }
    #[test]
    fn explicit_non_thinking_policy_never_downgrades_effort_or_crosses_provider() {
        let mut request = request();
        let mut body = json!({"model": "fixture"});
        ChatRequestPolicy::DeepSeekNonThinking
            .apply(&request, &mut body)
            .unwrap();
        assert_eq!(body["thinking"]["type"], "disabled");
        request.reasoning_effort = Some("high".into());
        let original = json!({"model": "fixture"});
        let mut body = original.clone();
        assert!(
            ChatRequestPolicy::DeepSeekNonThinking
                .apply(&request, &mut body)
                .is_err()
        );
        assert_eq!(body, original);
        request.reasoning_effort = None;
        request.model.provider_id = "qwen".into();
        assert!(
            ChatRequestPolicy::DeepSeekNonThinking
                .apply(&request, &mut body)
                .is_err()
        );
        ChatRequestPolicy::ProviderDefault
            .apply(&request, &mut body)
            .unwrap();
        assert_eq!(body, original);
    }

    #[test]
    fn json_object_policy_is_provider_neutral_and_preserves_other_fields() {
        let mut body = json!({"model": "fixture", "stream": true});
        ChatRequestPolicy::JsonObject
            .apply(&request(), &mut body)
            .unwrap();
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["stream"], true);
    }
}
