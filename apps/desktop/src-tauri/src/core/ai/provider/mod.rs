#[path = "cancellation.rs"]
mod cancellation;
#[cfg(any(test, feature = "development-data"))]
#[path = "deepseek.rs"]
mod deepseek;
#[path = "fixture.rs"]
mod fixture;
#[path = "model.rs"]
mod model;
#[path = "qwen_embedding.rs"]
mod qwen_embedding;

pub(crate) use crate::ai::shared_responses::SharedResponsesGenerationProvider;
pub(crate) use cancellation::CancellationSignal;
#[cfg(feature = "development-data")]
pub(crate) use deepseek::DeepSeekGenerationProvider;
#[cfg(test)]
pub(crate) use fixture::FixtureBehavior;
pub(crate) use fixture::FixtureGenerationProvider;
pub(crate) use model::{
    EmbeddingRequest, EmbeddingResponse, GenerationMessage, GenerationRequest, GenerationResponse,
    GenerationResponseFormat, GenerationRole, ProviderCapabilities, ProviderError,
    ProviderErrorKind, ProviderUsage, ThinkingMode,
};
pub(crate) use qwen_embedding::QwenEmbeddingProvider;

pub(crate) trait GenerationProvider {
    fn capabilities(&self) -> ProviderCapabilities;

    async fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: &CancellationSignal,
    ) -> Result<GenerationResponse, ProviderError>;
}

pub(crate) trait EmbeddingProvider {
    async fn embed(
        &self,
        request: &EmbeddingRequest,
        cancellation: &CancellationSignal,
    ) -> Result<EmbeddingResponse, ProviderError>;
}

pub(crate) fn capabilities_for(provider_id: &str) -> Option<ProviderCapabilities> {
    match provider_id {
        "fixture-generation" => Some(FixtureGenerationProvider::default().capabilities()),
        "deepseek" => Some(ProviderCapabilities {
            provider_id: "deepseek".to_owned(),
            provider_kind: "generation".to_owned(),
            is_cloud: true,
            supports_structured_output: true,
            supports_streaming: true,
            supports_cancellation: true,
            supports_usage: true,
            supports_vision_input: false,
        }),
        "openai" => Some(ProviderCapabilities {
            provider_id: "openai".to_owned(),
            provider_kind: "generation".to_owned(),
            is_cloud: true,
            supports_structured_output: true,
            supports_streaming: true,
            supports_cancellation: true,
            supports_usage: false,
            supports_vision_input: true,
        }),
        "qwen" => Some(ProviderCapabilities {
            provider_id: "qwen".to_owned(),
            provider_kind: "generation".to_owned(),
            is_cloud: true,
            supports_structured_output: true,
            supports_streaming: true,
            supports_cancellation: true,
            supports_usage: true,
            supports_vision_input: true,
        }),
        "qwen-embedding" => Some(ProviderCapabilities {
            provider_id: "qwen-embedding".to_owned(),
            provider_kind: "embedding".to_owned(),
            is_cloud: true,
            supports_structured_output: false,
            supports_streaming: false,
            supports_cancellation: true,
            supports_usage: true,
            supports_vision_input: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use qwen_embedding::EmbeddingHttpWire;
