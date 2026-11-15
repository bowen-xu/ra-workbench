pub(crate) use super::provider::EmbeddingResponse;
use std::time::Duration;

use rusqlite::Connection;

use super::{
    EmbeddingProviderConfiguration, JsonCredentialStore,
    provider::{
        CancellationSignal, EmbeddingProvider, EmbeddingRequest, ProviderError,
        QwenEmbeddingProvider,
    },
    resolve_embedding_provider_configuration,
};

/// Application-assembled access to the existing credential owner. Consumers
/// receive neither the credential store nor a resolved provider secret.
#[derive(Clone)]
pub(crate) struct EmbeddingAccess {
    credentials: JsonCredentialStore,
    #[cfg(test)]
    wire: Option<std::sync::Arc<dyn super::provider::EmbeddingHttpWire>>,
}

/// One request's configuration, frozen while borrowing the caller's preparation
/// connection. It deliberately has no serialization or secret-bearing accessors.
pub(crate) struct PreparedEmbedding {
    configuration: EmbeddingProviderConfiguration,
    #[cfg(test)]
    wire: Option<std::sync::Arc<dyn super::provider::EmbeddingHttpWire>>,
}

#[derive(Debug)]
pub(crate) enum EmbeddingCallError {
    Configuration,
    Provider(ProviderError),
}

impl EmbeddingAccess {
    pub(crate) fn new(credentials: JsonCredentialStore) -> Self {
        Self {
            credentials,
            #[cfg(test)]
            wire: None,
        }
    }

    #[cfg(all(test, unix, debug_assertions, feature = "development-data"))]
    pub(crate) fn for_test(
        credentials: JsonCredentialStore,
        wire: std::sync::Arc<dyn super::provider::EmbeddingHttpWire>,
    ) -> Self {
        Self {
            credentials,
            wire: Some(wire),
        }
    }
    pub(crate) fn prepare_in(
        &self,
        connection: &Connection,
    ) -> Result<PreparedEmbedding, EmbeddingCallError> {
        let configuration = resolve_embedding_provider_configuration(connection, &self.credentials)
            .map_err(|_| EmbeddingCallError::Configuration)?;
        Ok(PreparedEmbedding {
            configuration,
            #[cfg(test)]
            wire: self.wire.clone(),
        })
    }
}

impl PreparedEmbedding {
    pub(crate) fn provider_id(&self) -> &str {
        "qwen-embedding"
    }

    pub(crate) fn model_id(&self) -> &str {
        &self.configuration.model_id
    }

    pub(crate) async fn embed(
        self,
        inputs: Vec<String>,
        dimension: usize,
        cancellation: &CancellationSignal,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        let timeout = u64::try_from(self.configuration.timeout_ms)
            .map(Duration::from_millis)
            .map_err(|_| EmbeddingCallError::Configuration)?;
        let provider =
            QwenEmbeddingProvider::new(&self.configuration.base_url, self.configuration.credential)
                .map_err(EmbeddingCallError::Provider)?;
        #[cfg(test)]
        let provider = match self.wire {
            Some(wire) => provider.with_test_wire(wire),
            None => provider,
        };
        provider
            .embed(
                &EmbeddingRequest {
                    model_id: self.configuration.model_id,
                    inputs,
                    dimension,
                    timeout,
                },
                cancellation,
            )
            .await
            .map_err(EmbeddingCallError::Provider)
    }
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
