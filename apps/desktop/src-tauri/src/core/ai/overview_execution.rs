//! Paper overview/portrait orchestration over the existing shared run stages.
use std::time::Instant;
pub(super) mod native;

use crate::ai::{
    CancellationSignal,
    candidate_contracts::AiEvidenceBundle,
    error::AiError,
    generation_support::{AiRun, GenerationMessage, GenerationResponse, ProviderUsage, hash_text},
};

pub(crate) trait OverviewHost: Sync {
    type Model: Send + Sync;
    fn resolve_stage(&self) -> impl Future<Output = Result<String, AiError>> + Send;
    fn begin(
        &self,
        kind: &str,
        input: Option<String>,
    ) -> impl Future<Output = Result<String, AiError>> + Send;
    fn complete(
        &self,
        stage: &str,
        output: Option<String>,
        started: Instant,
    ) -> impl Future<Output = Result<(), AiError>> + Send;
    fn fail(
        &self,
        stage: &str,
        error: &AiError,
        started: Instant,
    ) -> impl Future<Output = Result<(), AiError>> + Send;
    fn progress(&self);
    fn validate_evidence(
        &self,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> impl Future<Output = Result<(), AiError>> + Send;
    fn messages(
        &self,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> impl Future<Output = Result<Vec<GenerationMessage>, AiError>> + Send;
    fn resolve_bundle(
        &self,
        run: &AiRun,
    ) -> impl Future<Output = Result<AiEvidenceBundle, AiError>> + Send;
    fn prepare_model(&self) -> impl Future<Output = Result<Self::Model, AiError>> + Send;
    fn generate(
        &self,
        model: &Self::Model,
        input_hash: Option<String>,
        messages: Vec<GenerationMessage>,
    ) -> impl Future<Output = Result<GenerationResponse, AiError>> + Send;
    fn persist_candidate(
        &self,
        run: &str,
        bundle: AiEvidenceBundle,
        content: String,
        retry: bool,
    ) -> impl Future<Output = Result<String, AiError>> + Send;
    fn finish(&self, usage: &ProviderUsage) -> impl Future<Output = Result<(), AiError>> + Send;
}

pub(crate) async fn execute_overview(
    host: &impl OverviewHost,
    run: &AiRun,
    signal: &CancellationSignal,
) -> Result<(), AiError> {
    let resolved_hash = host.resolve_stage().await?;
    let retrieval_started = Instant::now();
    let retrieval_stage = host.begin("retrieving", Some(resolved_hash)).await?;
    if signal.race(async {}).await.is_err() {
        host.fail(&retrieval_stage, &AiError::Canceled, retrieval_started)
            .await?;
        return Err(AiError::Canceled);
    }
    let bundle = match host.resolve_bundle(run).await {
        Ok(bundle) => bundle,
        Err(error) => {
            host.fail(&retrieval_stage, &error, retrieval_started)
                .await?;
            return Err(error);
        }
    };
    let evidence_hash = hash_text(
        &serde_json::to_string(&bundle)
            .map_err(|_| AiError::InvalidInput("无法规范化概要 Evidence Bundle".to_owned()))?,
    );
    host.complete(
        &retrieval_stage,
        Some(evidence_hash.clone()),
        retrieval_started,
    )
    .await?;
    host.progress();
    let extraction_started = Instant::now();
    let extraction_stage = host
        .begin("extracting_evidence", Some(evidence_hash))
        .await?;
    let validation = host.validate_evidence(run, &bundle).await;
    if let Err(error) = validation {
        host.fail(&extraction_stage, &error, extraction_started)
            .await?;
        return Err(error);
    }
    let extracted_hash = hash_text(&format!("overview-evidence:{}", bundle.id));
    host.complete(
        &extraction_stage,
        Some(extracted_hash.clone()),
        extraction_started,
    )
    .await?;
    host.progress();
    let model = host.prepare_model().await?;
    let messages = host.messages(run, &bundle).await?;
    let response = host
        .generate(&model, Some(extracted_hash), messages)
        .await?;
    let validation_started = Instant::now();
    let validation_stage = host
        .begin("validating", Some(hash_text(&response.content)))
        .await?;
    let candidate_hash = host
        .persist_candidate(
            &run.id,
            bundle,
            response.content.clone(),
            run.overview_retry.is_some(),
        )
        .await;
    let candidate_hash = match candidate_hash {
        Ok(hash) => hash,
        Err(error) => {
            host.fail(&validation_stage, &error, validation_started)
                .await?;
            return Err(error);
        }
    };
    host.complete(&validation_stage, Some(candidate_hash), validation_started)
        .await?;
    host.progress();
    host.finish(&response.usage).await
}
