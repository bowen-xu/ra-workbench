use reqwest::StatusCode;
#[cfg(feature = "development-data")]
use reqwest::{Client, header::RETRY_AFTER, redirect::Policy};
use serde::{Deserialize, Serialize};

use super::model::{
    GenerationRequest, GenerationResponse, GenerationResponseFormat, ProviderError,
    ProviderErrorKind, ProviderUsage,
};
#[cfg(feature = "development-data")]
use super::{GenerationProvider, cancellation::CancellationSignal, model::ProviderCapabilities};
#[cfg(feature = "development-data")]
use crate::ai::credential::ProviderSecret;

#[cfg(feature = "development-data")]
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[cfg(feature = "development-data")]
pub(crate) struct DeepSeekGenerationProvider {
    client: Client,
    endpoint: String,
    credential: ProviderSecret,
}

#[cfg(feature = "development-data")]
impl DeepSeekGenerationProvider {
    pub(crate) fn new(base_url: &str, credential: ProviderSecret) -> Result<Self, ProviderError> {
        let endpoint = match base_url.trim_end_matches('/') {
            "https://api.deepseek.com" => "https://api.deepseek.com/chat/completions",
            "https://api.deepseek.com/v1" => "https://api.deepseek.com/v1/chat/completions",
            _ => return Err(ProviderError::new(ProviderErrorKind::InvalidRequest)),
        };
        let client = Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(|_| ProviderError::new(ProviderErrorKind::TemporaryUnavailable))?;
        Ok(Self {
            client,
            endpoint: endpoint.to_owned(),
            credential,
        })
    }
}

#[cfg(feature = "development-data")]
impl GenerationProvider for DeepSeekGenerationProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            provider_id: "deepseek".to_owned(),
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
        if cancellation.is_cancelled() {
            return Err(ProviderError::new(ProviderErrorKind::Canceled));
        }
        let body = DeepSeekRequest::from_generation(request);
        let pending = self
            .client
            .post(&self.endpoint)
            .bearer_auth(self.credential.expose())
            .timeout(request.timeout)
            .json(&body)
            .send();
        let response = cancellation
            .race(pending)
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::Canceled))?
            .map_err(map_transport_error)?;
        if !response.status().is_success() {
            return Err(map_http_error(
                response.status(),
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
        }
        let bytes = cancellation
            .race(response.bytes())
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::Canceled))?
            .map_err(map_transport_error)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
        }
        let payload: DeepSeekResponse = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidResponse))?;
        payload.into_generation()
    }
}

#[derive(Serialize)]
struct DeepSeekRequest<'a> {
    model: &'a str,
    messages: Vec<DeepSeekMessage<'a>>,
    thinking: DeepSeekThinking<'a>,
    max_tokens: u32,
    response_format: DeepSeekResponseFormat<'a>,
    stream: bool,
}

impl<'a> DeepSeekRequest<'a> {
    fn from_generation(request: &'a GenerationRequest) -> Self {
        Self {
            model: &request.model_id,
            messages: request
                .messages
                .iter()
                .map(|message| DeepSeekMessage {
                    role: message.role.as_str(),
                    content: &message.content,
                })
                .collect(),
            thinking: DeepSeekThinking {
                kind: request.thinking_mode.as_str(),
            },
            max_tokens: request.max_output_tokens,
            response_format: DeepSeekResponseFormat {
                kind: match request.response_format {
                    GenerationResponseFormat::JsonObject => "json_object",
                },
            },
            stream: false,
        }
    }
}

#[derive(Serialize)]
struct DeepSeekMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct DeepSeekThinking<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Serialize)]
struct DeepSeekResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Deserialize)]
struct DeepSeekResponse {
    model: String,
    choices: Vec<DeepSeekChoice>,
    usage: Option<DeepSeekUsage>,
}

impl DeepSeekResponse {
    fn into_generation(self) -> Result<GenerationResponse, ProviderError> {
        if self.choices.len() != 1 {
            return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
        }
        let choice = self
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::new(ProviderErrorKind::InvalidResponse))?;
        let finish_error = match choice.finish_reason.as_str() {
            "stop" => None,
            "length" => Some(ProviderErrorKind::TruncatedOutput),
            "content_filter" => Some(ProviderErrorKind::ContentFiltered),
            "tool_calls" => Some(ProviderErrorKind::ForbiddenToolCall),
            "insufficient_system_resource" => Some(ProviderErrorKind::TemporaryUnavailable),
            _ => Some(ProviderErrorKind::InvalidResponse),
        };
        if let Some(kind) = finish_error {
            return Err(ProviderError::new(kind));
        }
        if choice
            .message
            .tool_calls
            .is_some_and(|calls| !calls.is_empty())
        {
            return Err(ProviderError::new(ProviderErrorKind::ForbiddenToolCall));
        }
        let content = choice.message.content.unwrap_or_default();
        if content.trim().is_empty() {
            return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
        }
        let usage = self.usage.unwrap_or_default();
        Ok(GenerationResponse {
            model_id: self.model,
            content,
            usage: ProviderUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_hit_tokens: usage.prompt_cache_hit_tokens,
                cache_miss_tokens: usage.prompt_cache_miss_tokens,
            },
        })
    }
}

#[derive(Deserialize)]
struct DeepSeekChoice {
    finish_reason: String,
    message: DeepSeekResponseMessage,
}

#[derive(Deserialize)]
struct DeepSeekResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Default, Deserialize)]
struct DeepSeekUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_cache_hit_tokens: Option<u64>,
    prompt_cache_miss_tokens: Option<u64>,
}

#[cfg(feature = "development-data")]
fn map_transport_error(error: reqwest::Error) -> ProviderError {
    ProviderError::new(if error.is_timeout() {
        ProviderErrorKind::Timeout
    } else {
        ProviderErrorKind::TemporaryUnavailable
    })
}

fn map_http_error(status: StatusCode, retry_after: Option<&str>) -> ProviderError {
    let kind = match status.as_u16() {
        400 | 422 => ProviderErrorKind::InvalidRequest,
        401 | 403 => ProviderErrorKind::Authentication,
        402 => ProviderErrorKind::InsufficientBalance,
        429 => ProviderErrorKind::RateLimited,
        500..=599 => ProviderErrorKind::TemporaryUnavailable,
        _ => ProviderErrorKind::InvalidResponse,
    };
    let retry_after_ms = retry_after
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000).min(300_000));
    ProviderError::with_retry_after(kind, retry_after_ms)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ai::provider::model::{GenerationMessage, GenerationRole, ThinkingMode};

    fn request() -> GenerationRequest {
        GenerationRequest {
            model_id: "deepseek-v4-flash".to_owned(),
            messages: vec![GenerationMessage {
                role: GenerationRole::User,
                content: "Return a synthetic JSON object.".to_owned(),
            }],
            max_output_tokens: 120,
            thinking_mode: ThinkingMode::Disabled,
            response_format: GenerationResponseFormat::JsonObject,
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn request_sets_thinking_and_json_mode_without_tools_or_streaming() {
        let value = serde_json::to_value(DeepSeekRequest::from_generation(&request()))
            .expect("serialize DeepSeek request");
        assert_eq!(value["thinking"]["type"], "disabled");
        assert_eq!(value["response_format"]["type"], "json_object");
        assert_eq!(value["stream"], false);
        assert!(value.get("tools").is_none());
        assert!(value.get("user_id").is_none());
    }

    #[test]
    fn response_decoder_uses_actual_model_and_usage_without_reasoning_content() {
        let payload: DeepSeekResponse = serde_json::from_value(serde_json::json!({
            "model": "deepseek-v4-flash-20260804",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "{\"ok\":true}",
                    "reasoning_content": "must not enter the normalized result"
                }
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 8,
                "prompt_cache_hit_tokens": 5,
                "prompt_cache_miss_tokens": 15
            }
        }))
        .expect("decode provider fixture");
        let response = payload.into_generation().expect("normalize response");
        assert_eq!(response.model_id, "deepseek-v4-flash-20260804");
        assert_eq!(response.content, r#"{"ok":true}"#);
        assert_eq!(response.usage.cache_hit_tokens, Some(5));
    }

    #[test]
    fn unsafe_completion_states_and_http_failures_have_stable_categories() {
        for (finish_reason, expected) in [
            ("length", ProviderErrorKind::TruncatedOutput),
            ("content_filter", ProviderErrorKind::ContentFiltered),
            ("tool_calls", ProviderErrorKind::ForbiddenToolCall),
            (
                "insufficient_system_resource",
                ProviderErrorKind::TemporaryUnavailable,
            ),
        ] {
            let payload: DeepSeekResponse = serde_json::from_value(serde_json::json!({
                "model": "deepseek-v4-flash",
                "choices": [{"finish_reason": finish_reason, "message": {"content": "ignored"}}]
            }))
            .expect("decode failure fixture");
            assert_eq!(
                payload.into_generation().expect_err("unsafe finish").kind,
                expected
            );
        }
        assert_eq!(
            map_http_error(StatusCode::UNAUTHORIZED, None).kind,
            ProviderErrorKind::Authentication
        );
        let limited = map_http_error(StatusCode::TOO_MANY_REQUESTS, Some("3"));
        assert_eq!(limited.kind, ProviderErrorKind::RateLimited);
        assert_eq!(limited.retry_after_ms, Some(3000));
        assert_eq!(
            map_http_error(StatusCode::SERVICE_UNAVAILABLE, None).kind,
            ProviderErrorKind::TemporaryUnavailable
        );
    }
}
