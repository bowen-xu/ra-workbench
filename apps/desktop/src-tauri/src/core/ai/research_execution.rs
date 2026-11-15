use super::overview_execution::OverviewHost;
use crate::ai::candidate_contracts::AiEvidenceBundle;
use crate::ai::{
    CancellationSignal,
    error::AiError,
    generation_support::{AiRun, hash_text},
};
use std::time::Instant;
pub(crate) async fn execute_research_task(
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
            .map_err(|_| AiError::InvalidInput("无法规范化研究 Evidence Bundle".to_owned()))?,
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
    if let Err(error) = host.validate_evidence(run, &bundle).await {
        host.fail(&extraction_stage, &error, extraction_started)
            .await?;
        return Err(error);
    }
    let extracted_hash = hash_text(&format!("research-evidence:{}", bundle.id));
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
    let candidate = host
        .persist_candidate(&run.id, bundle, response.content.clone(), false)
        .await;
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => {
            host.fail(&validation_stage, &error, validation_started)
                .await?;
            return Err(error);
        }
    };
    host.complete(&validation_stage, Some(candidate), validation_started)
        .await?;
    host.progress();
    host.finish(&response.usage).await
}

pub(crate) fn policy_input(
    run: &AiRun,
    bundle: &AiEvidenceBundle,
) -> research_workbench_native_sdk::analysis_workflow::research::ResearchPromptInputV1 {
    use research_workbench_native_sdk::analysis_workflow::research::{
        ResearchEvidenceItemV1, ResearchPromptInputV1,
    };
    ResearchPromptInputV1 {
        task_kind: run.task_kind.clone(),
        user_goal: run.user_goal.clone(),
        matrix_context: run.matrix_synthesis_context.clone(),
        validation_status: bundle.validation_status.clone(),
        items: bundle
            .items
            .iter()
            .map(|item| ResearchEvidenceItemV1 {
                citation_key: item.citation_key.clone(),
                paper_id: item.paper_id.clone(),
                paper_title: item.paper_title.clone(),
                quote_exact: item.quote_exact.clone(),
                context_text: item.context_text.clone(),
                content_sha256: item.content_sha256.clone(),
                page_index: item.page_index,
                validation_status: item.validation_status.clone(),
            })
            .collect(),
    }
}
