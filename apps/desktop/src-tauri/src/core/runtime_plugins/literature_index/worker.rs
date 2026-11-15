use super::model::SemanticIndexError;
use crate::{ai::CancellationSignal, semantic_engine::SemanticVectorStore};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultPoint {
    BeforeLanceWrite,
    AfterLanceWrite,
}

pub(crate) async fn run_embedding_job<H: super::host::IndexHost>(
    host: &H,
    job_id: &str,
    vector: Vec<f32>,
    cancellation: &CancellationSignal,
) -> Result<(), H::Error> {
    run_embedding_job_inner(host, job_id, vector, cancellation, None).await
}

async fn run_embedding_job_inner<H: super::host::IndexHost>(
    host: &H,
    job_id: &str,
    vector: Vec<f32>,
    cancellation: &CancellationSignal,
    fault_point: Option<FaultPoint>,
) -> Result<(), H::Error> {
    if cancellation.is_cancelled() {
        return Err(SemanticIndexError::Canceled.into());
    }
    let job = host.load_and_mark_running(job_id).await?;
    if vector.len() != job.dimension || vector.iter().any(|value| !value.is_finite()) {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::InvalidInput("embedding provider 返回的向量无效").into());
    }
    if fault_point == Some(FaultPoint::BeforeLanceWrite) {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::InvalidInput("fault before LanceDB write").into());
    }
    let lancedb = host
        .open(&job.relative_database_path, job.namespace, job.dimension)
        .await?;
    lancedb
        .ensure_ready()
        .await
        .map_err(SemanticIndexError::from)?;
    let mut row = job.row.clone();
    row.vector = vector;
    let vector_norm = row
        .vector
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !vector_norm.is_finite() {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::InvalidInput("embedding vector norm 无效").into());
    }
    if cancellation.is_cancelled() {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::Canceled.into());
    }
    let receipt = lancedb
        .upsert(&row)
        .await
        .map_err(SemanticIndexError::from)?;
    if fault_point == Some(FaultPoint::AfterLanceWrite) {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::InvalidInput("fault after LanceDB write").into());
    }
    if !lancedb
        .contains(&row.row_id, &row.input_sha256)
        .await
        .map_err(SemanticIndexError::from)?
    {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::EngineProtocol(
            "written vector could not be verified".to_owned(),
        )
        .into());
    }
    if cancellation.is_cancelled() {
        host.interrupt(job_id).await?;
        return Err(SemanticIndexError::Canceled.into());
    }
    host.ready(job, receipt.dataset_version, vector_norm).await
}
