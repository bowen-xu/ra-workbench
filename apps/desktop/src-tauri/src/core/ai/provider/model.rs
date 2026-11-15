use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub provider_id: String,
    pub provider_kind: String,
    pub is_cloud: bool,
    pub supports_structured_output: bool,
    pub supports_streaming: bool,
    pub supports_cancellation: bool,
    pub supports_usage: bool,
    pub supports_vision_input: bool,
}

#[derive(Clone)]
pub(crate) struct GenerationMessage {
    pub(crate) role: GenerationRole,
    pub(crate) content: String,
}

#[derive(Clone, Copy)]
pub(crate) enum GenerationRole {
    System,
    User,
}

impl GenerationRole {
    #[cfg(any(test, feature = "development-data"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }
}

#[derive(Clone)]
pub(crate) struct GenerationRequest {
    pub(crate) model_id: String,
    pub(crate) messages: Vec<GenerationMessage>,
    pub(crate) max_output_tokens: u32,
    #[cfg_attr(
        not(any(test, feature = "development-data")),
        expect(
            dead_code,
            reason = "legacy generation contract retained; shared resolver freezes own policy"
        )
    )]
    pub(crate) thinking_mode: ThinkingMode,
    #[cfg_attr(
        not(any(test, feature = "development-data")),
        expect(
            dead_code,
            reason = "legacy generation contract retained; shared resolver freezes own policy"
        )
    )]
    pub(crate) response_format: GenerationResponseFormat,
    pub(crate) timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct EmbeddingRequest {
    pub(crate) model_id: String,
    pub(crate) inputs: Vec<String>,
    pub(crate) dimension: usize,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Copy)]
pub(crate) enum ThinkingMode {
    Disabled,
    Enabled,
}

impl ThinkingMode {
    #[cfg(any(test, feature = "development-data"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum GenerationResponseFormat {
    JsonObject,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderUsage {
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) cache_hit_tokens: Option<u64>,
    pub(crate) cache_miss_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenerationResponse {
    pub(crate) model_id: String,
    pub(crate) content: String,
    pub(crate) usage: ProviderUsage,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmbeddingResponse {
    pub(crate) model_id: String,
    pub(crate) vectors: Vec<Vec<f32>>,
    pub(crate) usage: ProviderUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderErrorKind {
    Authentication,
    InsufficientBalance,
    InvalidRequest,
    RateLimited,
    TemporaryUnavailable,
    Timeout,
    Canceled,
    InvalidResponse,
    TruncatedOutput,
    ContentFiltered,
    ForbiddenToolCall,
}

impl ProviderErrorKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::InsufficientBalance => "insufficient_balance",
            Self::InvalidRequest => "invalid_request",
            Self::RateLimited => "rate_limited",
            Self::TemporaryUnavailable => "temporary_unavailable",
            Self::Timeout => "timeout",
            Self::Canceled => "canceled",
            Self::InvalidResponse => "invalid_response",
            Self::TruncatedOutput => "truncated_output",
            Self::ContentFiltered => "content_filtered",
            Self::ForbiddenToolCall => "forbidden_tool_call",
        }
    }

    #[cfg(test)]
    pub(crate) fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::TemporaryUnavailable | Self::Timeout
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderError {
    pub(crate) kind: ProviderErrorKind,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) model_error: Option<research_agent_core::model::ModelError>,
}

impl ProviderError {
    pub(crate) fn new(kind: ProviderErrorKind) -> Self {
        Self {
            kind,
            retry_after_ms: None,
            model_error: None,
        }
    }

    pub(crate) fn with_retry_after(kind: ProviderErrorKind, retry_after_ms: Option<u64>) -> Self {
        Self {
            kind,
            retry_after_ms,
            model_error: None,
        }
    }
}
