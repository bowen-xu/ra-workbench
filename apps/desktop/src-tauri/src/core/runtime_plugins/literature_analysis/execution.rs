//! Native policy uses the original shared Core analysis and Docling sequences.
use crate::{
    ai::{
        CancellationSignal,
        analysis_support::{AnalysisExecution, AnalysisHost, PreparedWorkflow},
        error::AiCommandError,
        paper_analysis_contracts::{
            DoclingPaperOverviewDraft, PaperAnalysisStartDraft, PaperAnalysisWorkflow,
        },
    },
    runtime_plugins::literature_trace::service::{AttemptFailure, LiteratureTraceService},
    storage::AppStore,
};
use std::sync::Arc;

pub(super) struct NativeAnalysisHost {
    pub store: AppStore,
    pub execution: AnalysisExecution,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
}
impl AnalysisHost for NativeAnalysisHost {
    async fn prepare_sources(&self, draft: &PaperAnalysisStartDraft) -> Result<(), AiCommandError> {
        self.execution
            .prepare_sources(&draft.paper_id, &draft.attachment_ids)
            .await
    }
    async fn prepare(
        &self,
        draft: PaperAnalysisStartDraft,
    ) -> Result<PreparedWorkflow, AiCommandError> {
        self.service
            .prepare_analysis_workflow(self.store.clone(), self.key.clone(), draft)
            .await
    }
    async fn overview(
        &self,
        draft: DoclingPaperOverviewDraft,
        signal: CancellationSignal,
    ) -> Result<String, AiCommandError> {
        self.execution.overview(draft, signal).await
    }
    async fn complete(
        &self,
        workflow: String,
        run: String,
    ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
        self.service
            .complete_analysis_overview(self.store.clone(), self.key.clone(), workflow, run)
            .await
    }
    async fn failure(
        &self,
        workflow: String,
        code: String,
        message: String,
    ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
        self.service
            .fail_analysis_attempt(
                self.store.clone(),
                self.key.clone(),
                AttemptFailure {
                    workflow_id: workflow,
                    turn_kind: "paper_overview".into(),
                    error_code: code,
                    error_message: message,
                },
            )
            .await
            .map_err(AiCommandError::from_analysis_workflow)
    }
}
