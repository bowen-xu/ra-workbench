//! Original task runners consume the selected finite policy and Core execution.
use super::*;
use crate::{
    ai::generation_support::{OverviewExecution, OverviewModel},
    runtime_plugins::literature_trace::binding::NoteTraceBinding,
    storage::AppStore,
};

pub(in crate::ai) struct NativeOverviewHost<'a> {
    pub execution: OverviewExecution<'a>,
    pub store: &'a AppStore,
    pub binding: NoteTraceBinding,
}
impl OverviewHost for NativeOverviewHost<'_> {
    type Model = OverviewModel;
    async fn resolve_stage(&self) -> Result<String, AiError> {
        self.execution.resolve_stage().await
    }
    async fn begin(&self, kind: &str, input: Option<String>) -> Result<String, AiError> {
        self.execution.begin(kind, input).await
    }
    async fn complete(
        &self,
        stage: &str,
        output: Option<String>,
        started: Instant,
    ) -> Result<(), AiError> {
        self.execution.complete(stage, output, started).await
    }
    async fn fail(&self, stage: &str, error: &AiError, started: Instant) -> Result<(), AiError> {
        self.execution.fail(stage, error, started).await
    }
    fn progress(&self) {
        self.execution.progress()
    }
    async fn validate_evidence(
        &self,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> Result<(), AiError> {
        if matches!(
            run.task_kind.as_str(),
            "reference_extraction" | "matrix_synthesis"
        ) {
            return self
                .binding
                .service
                .validate_research_evidence(&self.binding.package_key, run, bundle)
                .await;
        }
        if run.task_kind == "paper_suggestions" {
            return crate::ai::generation_support::validate_evidence_bundle(bundle, None);
        }
        self.binding
            .service
            .validate_portrait_evidence(&self.binding.package_key, bundle)
            .await
    }
    async fn messages(
        &self,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> Result<Vec<GenerationMessage>, AiError> {
        if matches!(
            run.task_kind.as_str(),
            "reference_extraction" | "matrix_synthesis"
        ) {
            return self
                .binding
                .service
                .research_messages(&self.binding.package_key, run, bundle)
                .await;
        }
        if run.task_kind == "paper_suggestions" {
            return self
                .binding
                .service
                .suggestion_messages(&self.binding.package_key, run, bundle)
                .await;
        }
        // The original portrait request is empty here. The shared submission
        // prepares the actual Native task prompt and scoped file-tool input.
        Ok(Vec::new())
    }
    async fn resolve_bundle(&self, run: &AiRun) -> Result<AiEvidenceBundle, AiError> {
        if run.task_kind == "matrix_synthesis" {
            return self
                .binding
                .service
                .persist_matrix_task_evidence(
                    self.store.clone(),
                    self.binding.package_key.clone(),
                    run.id.clone(),
                )
                .await;
        }
        if run.task_kind == "reference_extraction" {
            return self
                .binding
                .service
                .persist_selection_task_evidence(
                    self.store.clone(),
                    self.binding.package_key.clone(),
                    run.id.clone(),
                )
                .await;
        }
        if run.task_kind == "paper_suggestions" {
            return self
                .binding
                .service
                .persist_page_task_evidence(
                    self.store.clone(),
                    self.binding.package_key.clone(),
                    run.id.clone(),
                )
                .await;
        }
        self.binding
            .service
            .portrait_evidence(
                self.store.clone(),
                self.binding.package_key.clone(),
                run.clone(),
            )
            .await
    }
    async fn prepare_model(&self) -> Result<OverviewModel, AiError> {
        self.execution.prepare_model().await
    }
    async fn generate(
        &self,
        model: &OverviewModel,
        hash: Option<String>,
        messages: Vec<GenerationMessage>,
    ) -> Result<GenerationResponse, AiError> {
        self.execution.generate(model, hash, messages).await
    }
    async fn persist_candidate(
        &self,
        run: &str,
        bundle: AiEvidenceBundle,
        content: String,
        _retry: bool,
    ) -> Result<String, AiError> {
        self.binding
            .service
            .save_analysis_candidate(
                self.store.clone(),
                self.binding.package_key.clone(),
                run.into(),
                bundle,
                content,
            )
            .await
            .map(|candidate| candidate.payload_hash().to_owned())
    }
    async fn finish(&self, usage: &ProviderUsage) -> Result<(), AiError> {
        self.execution.finish(usage).await
    }
}
