use reqwest::{Client, StatusCode, Url, header::RETRY_AFTER, redirect::Policy};
use serde::{Deserialize, Serialize};

use super::{
    EmbeddingProvider,
    cancellation::CancellationSignal,
    model::{EmbeddingRequest, EmbeddingResponse, ProviderError, ProviderErrorKind, ProviderUsage},
};
use crate::ai::credential::ProviderSecret;

#[cfg(test)]
pub(crate) trait EmbeddingHttpWire: Send + Sync {
    fn respond(
        &self,
        request: reqwest::Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<reqwest::Response, ProviderError>> + Send + '_>,
    >;
}

const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct QwenEmbeddingProvider {
    client: Client,
    endpoint: Url,
    credential: ProviderSecret,
    #[cfg(test)]
    wire: Option<std::sync::Arc<dyn EmbeddingHttpWire>>,
}

impl QwenEmbeddingProvider {
    pub(crate) fn new(base_url: &str, credential: ProviderSecret) -> Result<Self, ProviderError> {
        let endpoint = embedding_endpoint(base_url)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(|_| ProviderError::new(ProviderErrorKind::TemporaryUnavailable))?;
        Ok(Self {
            client,
            endpoint,
            credential,
            #[cfg(test)]
            wire: None,
        })
    }
    #[cfg(test)]
    pub(crate) fn with_test_wire(mut self, wire: std::sync::Arc<dyn EmbeddingHttpWire>) -> Self {
        self.wire = Some(wire);
        self
    }
    async fn send(
        &self,
        pending: reqwest::RequestBuilder,
        cancellation: &CancellationSignal,
    ) -> Result<reqwest::Response, ProviderError> {
        #[cfg(test)]
        if let Some(wire) = &self.wire {
            let request = pending.build().map_err(map_transport_error)?;
            return cancellation
                .race(wire.respond(request))
                .await
                .map_err(|_| ProviderError::new(ProviderErrorKind::Canceled))?;
        }
        cancellation
            .race(pending.send())
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::Canceled))?
            .map_err(map_transport_error)
    }
}

impl EmbeddingProvider for QwenEmbeddingProvider {
    async fn embed(
        &self,
        request: &EmbeddingRequest,
        cancellation: &CancellationSignal,
    ) -> Result<EmbeddingResponse, ProviderError> {
        validate_request(request)?;
        if cancellation.is_cancelled() {
            return Err(ProviderError::new(ProviderErrorKind::Canceled));
        }
        let body = QwenEmbeddingRequest {
            model: &request.model_id,
            input: &request.inputs,
            dimensions: request.dimension,
            encoding_format: "float",
        };
        let pending = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.credential.expose())
            .timeout(request.timeout)
            .json(&body);
        let response = self.send(pending, cancellation).await?;
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
        let payload: QwenEmbeddingResponse = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidResponse))?;
        payload.normalize(request.inputs.len(), request.dimension)
    }
}

#[derive(Serialize)]
struct QwenEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
    dimensions: usize,
    encoding_format: &'static str,
}

#[derive(Deserialize)]
struct QwenEmbeddingResponse {
    model: String,
    data: Vec<QwenEmbeddingData>,
    usage: Option<QwenEmbeddingUsage>,
}

impl QwenEmbeddingResponse {
    fn normalize(
        self,
        expected_count: usize,
        expected_dimension: usize,
    ) -> Result<EmbeddingResponse, ProviderError> {
        if self.data.len() != expected_count {
            return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
        }
        let mut vectors = vec![None; expected_count];
        for item in self.data {
            if item.index >= expected_count
                || item.embedding.len() != expected_dimension
                || item.embedding.iter().any(|value| !value.is_finite())
                || vectors[item.index].is_some()
            {
                return Err(ProviderError::new(ProviderErrorKind::InvalidResponse));
            }
            vectors[item.index] = Some(item.embedding);
        }
        let vectors = vectors
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| ProviderError::new(ProviderErrorKind::InvalidResponse))?;
        let usage = self.usage.unwrap_or_default();
        Ok(EmbeddingResponse {
            model_id: self.model,
            vectors,
            usage: ProviderUsage {
                input_tokens: usage.prompt_tokens.or(usage.total_tokens),
                ..ProviderUsage::default()
            },
        })
    }
}

#[derive(Deserialize)]
struct QwenEmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Default, Deserialize)]
struct QwenEmbeddingUsage {
    prompt_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

fn embedding_endpoint(base_url: &str) -> Result<Url, ProviderError> {
    let mut endpoint = Url::parse(base_url.trim())
        .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
    let host = endpoint.host_str().unwrap_or_default();
    if endpoint.scheme() != "https"
        || !host.ends_with(".aliyuncs.com")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.path().trim_end_matches('/') != "/compatible-mode/v1"
    {
        return Err(ProviderError::new(ProviderErrorKind::InvalidRequest));
    }
    endpoint.set_path("/compatible-mode/v1/embeddings");
    Ok(endpoint)
}

fn validate_request(request: &EmbeddingRequest) -> Result<(), ProviderError> {
    if request.model_id.trim().is_empty()
        || request.model_id.len() > 200
        || request.inputs.is_empty()
        || request.inputs.len() > 20
        || request.inputs.iter().any(|input| input.trim().is_empty())
        || request.dimension == 0
        || request.dimension > 4096
    {
        return Err(ProviderError::new(ProviderErrorKind::InvalidRequest));
    }
    Ok(())
}

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
        .map(|seconds| seconds.saturating_mul(1_000).min(300_000));
    ProviderError::with_retry_after(kind, retry_after_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_official_compatible_endpoint() {
        let valid =
            embedding_endpoint("https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1")
                .unwrap();
        assert_eq!(valid.path(), "/compatible-mode/v1/embeddings");
        assert!(embedding_endpoint("https://example.com/compatible-mode/v1").is_err());
        assert!(
            embedding_endpoint("https://workspace.cn-beijing.maas.aliyuncs.com/api/v1").is_err()
        );
    }

    #[test]
    fn response_restores_input_order_and_rejects_invalid_vectors() {
        let payload: QwenEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "model": "text-embedding-v4",
            "data": [
                {"index": 1, "embedding": [0.0, 1.0]},
                {"index": 0, "embedding": [1.0, 0.0]}
            ],
            "usage": {"prompt_tokens": 4, "total_tokens": 4}
        }))
        .unwrap();
        let response = payload.normalize(2, 2).unwrap();
        assert_eq!(response.vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!(response.usage.input_tokens, Some(4));

        let invalid: QwenEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "model": "text-embedding-v4",
            "data": [{"index": 0, "embedding": [1.0]}]
        }))
        .unwrap();
        assert!(invalid.normalize(1, 2).is_err());
    }
}
