use super::{model::SemanticIndexError, worker::run_embedding_job};
use crate::ai::{
    CancellationSignal, EmbeddingAccess, EmbeddingCallError, ProviderError, ProviderErrorKind,
};
use std::sync::{Arc, Mutex};
#[derive(Clone, Default)]
pub(crate) struct SemanticIndexManager {
    active: Arc<Mutex<Option<ActiveJob>>>,
}

#[derive(Clone)]
struct ActiveJob {
    job_id: String,
    cancellation: CancellationSignal,
}

pub(crate) struct IndexRegistration {
    manager: SemanticIndexManager,
    job_id: String,
    signal: CancellationSignal,
}
impl IndexRegistration {
    pub(crate) fn signal(&self) -> CancellationSignal {
        self.signal.clone()
    }
}
impl Drop for IndexRegistration {
    fn drop(&mut self) {
        self.manager.release(&self.job_id);
    }
}

impl SemanticIndexManager {
    pub(crate) fn register_owned(
        &self,
        job_id: &str,
    ) -> Result<IndexRegistration, SemanticIndexError> {
        let signal = self.register(job_id)?;
        Ok(IndexRegistration {
            manager: self.clone(),
            job_id: job_id.into(),
            signal,
        })
    }
    pub(crate) fn register(&self, job_id: &str) -> Result<CancellationSignal, SemanticIndexError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| SemanticIndexError::WorkerUnavailable)?;
        if let Some(job) = active.as_ref() {
            if job.job_id == job_id {
                return Ok(job.cancellation.clone());
            }
            return Err(SemanticIndexError::InvalidInput(
                "当前已有 semantic index job 运行",
            ));
        }
        let cancellation = CancellationSignal::default();
        *active = Some(ActiveJob {
            job_id: job_id.to_owned(),
            cancellation: cancellation.clone(),
        });
        Ok(cancellation)
    }

    pub(crate) fn cancel(&self, job_id: &str) -> Result<bool, SemanticIndexError> {
        let active = self
            .active
            .lock()
            .map_err(|_| SemanticIndexError::WorkerUnavailable)?;
        if let Some(job) = active.as_ref().filter(|job| job.job_id == job_id) {
            job.cancellation.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn remove(&self, job_id: &str) {
        if let Ok(mut active) = self.active.lock()
            && active.as_ref().is_some_and(|job| job.job_id == job_id)
        {
            *active = None;
        }
    }

    pub(crate) fn release(&self, job_id: &str) {
        self.remove(job_id);
    }
}

pub(crate) async fn execute_job<H: super::host::IndexHost>(
    host: &H,
    embedding: &EmbeddingAccess,
    job_id: &str,
    cancellation: &CancellationSignal,
) -> Result<(), H::Error> {
    let (job, configuration) = host.prepare_job(embedding, job_id).await?;
    if job.provider_id != configuration.provider_id() || configuration.model_id() != job.model_id {
        return Err(SemanticIndexError::Provider("embedding_space_mismatch").into());
    }
    let response = configuration
        .embed(vec![job.rendered_text.clone()], job.dimension, cancellation)
        .await
        .map_err(|error| match error {
            EmbeddingCallError::Configuration => SemanticIndexError::Provider("configuration"),
            EmbeddingCallError::Provider(error) => provider_failure(error),
        })?;
    let vector = response
        .vectors
        .into_iter()
        .next()
        .ok_or(SemanticIndexError::Provider("invalid_response"))?;
    run_embedding_job(host, job_id, vector, cancellation).await
}

fn provider_failure(error: ProviderError) -> SemanticIndexError {
    if error.kind == ProviderErrorKind::Canceled {
        SemanticIndexError::Canceled
    } else {
        SemanticIndexError::Provider(error.kind.code())
    }
}

pub(crate) fn error_code(error: &SemanticIndexError) -> &'static str {
    match error {
        SemanticIndexError::Canceled => "canceled",
        SemanticIndexError::Provider(code) => code,
        SemanticIndexError::InvalidInput(_) => "invalid_input",
        SemanticIndexError::EngineLoad(_)
        | SemanticIndexError::EngineAbiMismatch
        | SemanticIndexError::EngineProtocol(_)
        | SemanticIndexError::EngineOperation { .. } => "semantic_engine_failure",
        SemanticIndexError::Database(_) | SemanticIndexError::Storage(_) => "storage_error",
        SemanticIndexError::WorkerUnavailable => "worker_unavailable",
    }
}
