//! The original DOI HTTP mechanism. Translation and error wording stay outside.
use reqwest::{Client, StatusCode, Url, header::CONTENT_TYPE, redirect::Policy};
use research_workbench_native_sdk::doi_metadata::{DoiHttpBodyV1, DoiHttpErrorV1};
use std::time::Duration;

/// One client is retained for the resolver and optional publisher read. Default
/// proxy matching deliberately uses the owning App process's environment.
pub(crate) struct MetadataHttp {
    client: Client,
}

impl MetadataHttp {
    pub(crate) fn new() -> Result<Self, String> {
        Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(Policy::limited(5))
            .build()
            .map(|client| Self { client })
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn get(
        &self,
        resolver: Url,
        accept: &str,
        limit: usize,
    ) -> Result<DoiHttpBodyV1, DoiHttpErrorV1> {
        let response = self
            .client
            .get(resolver)
            .header("Accept", accept)
            .header(
                "User-Agent",
                "ResearchAssistant/0.1 (local desktop research tool)",
            )
            .send()
            .await
            .map_err(|error| DoiHttpErrorV1::Send {
                cause: error.to_string(),
            })?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(DoiHttpErrorV1::NotFound {});
        }
        let status = status.as_u16();
        let response = response
            .error_for_status()
            .map_err(|error| DoiHttpErrorV1::Status {
                status,
                cause: error.to_string(),
            })?;
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(DoiHttpErrorV1::TooLarge { status });
        }
        let final_https = response.url().scheme() == "https";
        // Preserve the original full-read then actual-length check, including
        // chunked bodies. This migration does not introduce a streaming cutoff.
        let bytes = response
            .bytes()
            .await
            .map_err(|error| DoiHttpErrorV1::Read {
                status,
                cause: error.to_string(),
            })?;
        if bytes.len() > limit {
            return Err(DoiHttpErrorV1::TooLarge { status });
        }
        let body = String::from_utf8(bytes.to_vec()).map_err(|error| DoiHttpErrorV1::Utf8 {
            status,
            cause: error.to_string(),
        })?;
        Ok(DoiHttpBodyV1 {
            content_type,
            body,
            final_https,
        })
    }
}

#[cfg(test)]
mod tests;
