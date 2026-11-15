//! Fixed-host, bounded and revocable metadata GET. No prompt/model execution.
use super::*;
use std::time::Duration;
use std::{future::Future, pin::Pin};
pub(super) type FetchResult = Result<(Vec<CatalogModel>, Option<String>), AiError>;
pub(super) trait CatalogTransport: Send + Sync {
    fn fetch<'a>(
        &'a self,
        lease: &'a ChatGptLease,
        previous: Option<&'a CatalogCache>,
        version: &'a str,
    ) -> Pin<Box<dyn Future<Output = FetchResult> + Send + 'a>>;
}
pub(super) struct HttpCatalogTransport;
#[cfg(test)]
pub(super) struct NoNetwork;
#[cfg(test)]
impl CatalogTransport for NoNetwork {
    fn fetch<'a>(
        &'a self,
        _: &'a ChatGptLease,
        _: Option<&'a CatalogCache>,
        _: &'a str,
    ) -> Pin<Box<dyn Future<Output = FetchResult> + Send + 'a>> {
        Box::pin(async { panic!("cached fixture must not contact the provider") })
    }
}
impl CatalogTransport for HttpCatalogTransport {
    fn fetch<'a>(
        &'a self,
        lease: &'a ChatGptLease,
        previous: Option<&'a CatalogCache>,
        version: &'a str,
    ) -> Pin<Box<dyn Future<Output = FetchResult> + Send + 'a>> {
        Box::pin(fetch(lease, previous, version))
    }
}
async fn fetch(
    lease: &ChatGptLease,
    previous: Option<&CatalogCache>,
    version: &str,
) -> Result<(Vec<CatalogModel>, Option<String>), AiError> {
    let operation = async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let mut request = client
            .get(MODELS_URL)
            .query(&[("client_version", version)])
            .header("originator", "research_assistant")
            .build()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        if let Some(etag) = previous.and_then(|cache| cache.etag.as_ref())
            && let Ok(header) = reqwest::header::HeaderValue::from_str(etag)
        {
            request
                .headers_mut()
                .insert(reqwest::header::IF_NONE_MATCH, header);
        }
        lease
            .authorize_catalog(&mut request, version)
            .map_err(|_| AiError::Provider("authentication"))?;
        let mut response = client
            .execute(request)
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AiError::Provider("authentication"));
        }
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return previous
                .map(|cache| (cache.models.clone(), cache.etag.clone()))
                .ok_or(AiError::Provider("invalid_response"));
        }
        if !status.is_success() {
            return Err(AiError::TemporaryUnavailable);
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BYTES as u64)
        {
            return Err(AiError::Provider("invalid_response"));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 1024)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err(AiError::Provider("invalid_response"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((decode_models(&bytes)?, etag))
    };
    tokio::select! {
        _ = lease.revoked() => Err(AiError::Provider("authentication")),
        result = tokio::time::timeout(Duration::from_secs(10), operation) => result.map_err(|_| AiError::Timeout)?,
    }
}

pub(super) fn decode_models(bytes: &[u8]) -> Result<Vec<CatalogModel>, AiError> {
    if bytes.len() > MAX_BYTES {
        return Err(AiError::Provider("invalid_response"));
    }
    let catalog: WireCatalog =
        serde_json::from_slice(bytes).map_err(|_| AiError::Provider("invalid_response"))?;
    model::validate(&catalog.models)?;
    Ok(catalog.models)
}
