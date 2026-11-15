use crate::agent::model::{AgentHost, AgentModelResolver, AgentModelSelection};

use super::{
    paper_reading::{ProviderInput, ProviderMessage, ProviderMessageRole},
    paper_reading_shared_transport::call_prepared_text,
    provider::{
        CancellationSignal, GenerationProvider, GenerationRequest, GenerationResponse,
        ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderUsage,
    },
};

const BOUNDED_INSTRUCTIONS: &str = "You are a bounded research-text generation engine. Return only the JSON requested by the messages below. Do not invoke any tool.";

#[derive(Clone)]
pub(crate) struct SharedResponsesGenerationProvider {
    models: AgentModelResolver,
    reasoning_effort: Option<String>,
}

impl SharedResponsesGenerationProvider {
    pub(crate) fn new(models: AgentModelResolver, reasoning_effort: Option<String>) -> Self {
        Self {
            models,
            reasoning_effort,
        }
    }
}

/// Compatibility name for legacy AI-run settings. The implementation uses the
/// shared Responses transport and never starts the Codex CLI or app-server.
#[allow(dead_code)]
pub(crate) type CodexCliGenerationProvider = SharedResponsesGenerationProvider;

impl GenerationProvider for SharedResponsesGenerationProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            provider_id: "openai".to_owned(),
            provider_kind: "generation".to_owned(),
            is_cloud: true,
            supports_structured_output: true,
            supports_streaming: true,
            supports_cancellation: true,
            supports_usage: true,
            supports_vision_input: false,
        }
    }

    async fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: &CancellationSignal,
    ) -> Result<GenerationResponse, ProviderError> {
        let prepared = self
            .models
            .prepare(
                AgentHost::Review,
                Some(AgentModelSelection {
                    provider_id: "openai".to_owned(),
                    model_id: request.model_id.clone(),
                    reasoning_effort: self.reasoning_effort.clone(),
                    service_tier: None,
                }),
            )
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
        let response = tokio::time::timeout(
            request.timeout,
            call_prepared_text(
                prepared,
                provider_input(request),
                true,
                request.max_output_tokens,
                Some(cancellation.clone()),
            ),
        )
        .await
        .map_err(|_| ProviderError::new(ProviderErrorKind::Timeout))?
        .map_err(provider_error)?;
        Ok(GenerationResponse {
            model_id: request.model_id.clone(),
            content: response.content,
            usage: ProviderUsage {
                input_tokens: response
                    .input_tokens
                    .and_then(|value| u64::try_from(value).ok()),
                output_tokens: response
                    .output_tokens
                    .and_then(|value| u64::try_from(value).ok()),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            },
        })
    }
}

pub(crate) fn provider_input(request: &GenerationRequest) -> ProviderInput {
    provider_messages_input(&request.messages)
}

pub(crate) fn provider_messages_input(
    source_messages: &[super::provider::GenerationMessage],
) -> ProviderInput {
    let mut system_text = if source_messages.is_empty() {
        String::new()
    } else {
        BOUNDED_INSTRUCTIONS.to_owned()
    };
    let mut messages = Vec::new();
    for message in source_messages {
        match message.role {
            super::provider::GenerationRole::System => {
                system_text.push_str("\n\n");
                system_text.push_str(&message.content);
            }
            super::provider::GenerationRole::User => messages.push(ProviderMessage {
                role: ProviderMessageRole::User,
                text: message.content.clone(),
                images: vec![],
            }),
        }
    }
    ProviderInput {
        audit_text: "shared-agent-responses:text-generation".to_owned(),
        system_text: (!system_text.is_empty()).then_some(system_text),
        messages,
    }
}

pub(crate) fn provider_error(error: super::error::AiError) -> ProviderError {
    let kind = match error.code() {
        "authentication" | "credential_missing" | "credential_invalid" => {
            ProviderErrorKind::Authentication
        }
        "rate_limited" => ProviderErrorKind::RateLimited,
        "invalid_input" | "invalid_request" => ProviderErrorKind::InvalidRequest,
        "timeout" => ProviderErrorKind::Timeout,
        "canceled" => ProviderErrorKind::Canceled,
        "truncated_output" => ProviderErrorKind::TruncatedOutput,
        "content_filtered" => ProviderErrorKind::ContentFiltered,
        "forbidden_tool_call" => ProviderErrorKind::ForbiddenToolCall,
        "invalid_response" => ProviderErrorKind::InvalidResponse,
        _ => ProviderErrorKind::TemporaryUnavailable,
    };
    let mut result = ProviderError::new(kind);
    if let super::error::AiError::Model { cause, .. } = error {
        result.model_error = Some(cause);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn managed_tasks_have_no_legacy_system_prompt_or_excerpt_message() {
        let request = GenerationRequest {
            model_id: "fixture".into(),
            messages: vec![],
            max_output_tokens: 64,
            thinking_mode: super::super::provider::ThinkingMode::Disabled,
            response_format: super::super::provider::GenerationResponseFormat::JsonObject,
            timeout: Duration::from_secs(30),
        };
        let input = provider_input(&request);
        assert!(input.system_text.is_none());
        assert!(input.messages.is_empty());
    }

    #[test]
    fn model_diagnostic_survives_the_legacy_provider_adapter() {
        use research_agent_core::model::ModelError;
        for cause in [
            ModelError::HttpAuthentication(401),
            ModelError::HttpAuthentication(403),
            ModelError::HttpRateLimited(429),
            ModelError::ConnectionFailed,
        ] {
            let error = super::super::paper_reading_shared_transport::map_model_error(cause);
            let mapped = provider_error(error);
            assert_eq!(mapped.model_error, Some(cause));
        }
    }

    #[test]
    fn legacy_generation_input_is_a_thin_shared_transport_adapter() {
        let input = provider_input(&GenerationRequest {
            model_id: "gpt-5.6-sol".into(),
            messages: vec![
                super::super::provider::GenerationMessage {
                    role: super::super::provider::GenerationRole::System,
                    content: "Use schema v1".into(),
                },
                super::super::provider::GenerationMessage {
                    role: super::super::provider::GenerationRole::User,
                    content: "Return JSON".into(),
                },
            ],
            max_output_tokens: 64,
            thinking_mode: super::super::provider::ThinkingMode::Disabled,
            response_format: super::super::provider::GenerationResponseFormat::JsonObject,
            timeout: Duration::from_secs(30),
        });
        assert_eq!(input.messages.len(), 1);
        assert!(
            input
                .system_text
                .as_deref()
                .unwrap()
                .contains("Use schema v1")
        );
        assert_eq!(input.messages[0].text, "Return JSON");
        assert!(
            input
                .messages
                .iter()
                .all(|message| message.images.is_empty())
        );
    }
}
