//! Original analysis sequence shared by static and selected Native composition.
use super::{AiCommandError, AnalysisSupportError};
use crate::ai::{
    CancellationSignal,
    paper_analysis_contracts::{
        DoclingPaperOverviewDraft, PaperAnalysisStartDraft, PaperAnalysisWorkflow,
    },
};

pub(crate) struct PreparedWorkflow {
    pub workflow: PaperAnalysisWorkflow,
    pub overview_draft: DoclingPaperOverviewDraft,
}

pub(crate) trait AnalysisHost: Sync {
    fn prepare_sources(
        &self,
        draft: &PaperAnalysisStartDraft,
    ) -> impl Future<Output = Result<(), AiCommandError>> + Send;
    fn prepare(
        &self,
        draft: PaperAnalysisStartDraft,
    ) -> impl Future<Output = Result<PreparedWorkflow, AiCommandError>> + Send;
    fn overview(
        &self,
        draft: DoclingPaperOverviewDraft,
        cancellation: CancellationSignal,
    ) -> impl Future<Output = Result<String, AiCommandError>> + Send;
    fn complete(
        &self,
        workflow: String,
        run: String,
    ) -> impl Future<Output = Result<PaperAnalysisWorkflow, AiCommandError>> + Send;
    fn failure(
        &self,
        workflow: String,
        code: String,
        message: String,
    ) -> impl Future<Output = Result<PaperAnalysisWorkflow, AiCommandError>> + Send;
}
pub(crate) async fn start_workflow(
    host: &impl AnalysisHost,
    draft: PaperAnalysisStartDraft,
    cancellation: CancellationSignal,
) -> Result<PaperAnalysisWorkflow, AiCommandError> {
    cancellation
        .race(host.prepare_sources(&draft))
        .await
        .map_err(|_| AnalysisSupportError::canceled().into_command_error())??;
    let prepared = host.prepare(draft).await?;
    if cancellation.is_cancelled() {
        let error = AnalysisSupportError::canceled().into_command_error();
        return host
            .failure(
                prepared.workflow.id,
                error.code().into(),
                error.message().into(),
            )
            .await;
    }
    let id = prepared.workflow.id;
    match host.overview(prepared.overview_draft, cancellation).await {
        Ok(run) => host.complete(id, run).await,
        Err(error) => {
            if matches!(
                error.code(),
                "data_operation_outcome_unknown" | "ai_run_terminal_outcome_unknown"
            ) {
                return Err(error);
            }
            host.failure(id, error.code().into(), error.message().into())
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::error::AiError, ai::paper_analysis_contracts::*,
        runtime_plugins::literature_analysis::manager::PaperAnalysisWorkflowManager,
    };
    use std::sync::Mutex;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Branch {
        Before,
        After,
        CreateUnknown,
        TerminalUnknown,
        Failure,
        Success,
    }
    struct Host {
        branch: Branch,
        signal: CancellationSignal,
        manager: PaperAnalysisWorkflowManager,
        events: Mutex<Vec<&'static str>>,
    }
    fn workflow() -> PaperAnalysisWorkflow {
        PaperAnalysisWorkflow {
            id: "workflow".into(),
            active_client_request_id: None,
            conversation_id: Some(uuid::Uuid::nil()),
            paper_id: "paper".into(),
            context_snapshot_id: "snapshot".into(),
            status: "preparing".into(),
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            reasoning_effort: None,
            prompt_set_version: "version".into(),
            active_turn_kind: None,
            revision: 1,
            supersedes_workflow_id: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            completed_at: None,
            turns: vec![],
        }
    }
    impl Host {
        fn stage(&self, name: &'static str) {
            assert_eq!(
                self.manager.active_request("paper").unwrap().as_deref(),
                Some("request")
            );
            assert!(self.manager.register_for_paper("other", "paper").is_err());
            self.events.lock().unwrap().push(name);
        }
    }
    impl AnalysisHost for Host {
        async fn prepare_sources(
            &self,
            draft: &PaperAnalysisStartDraft,
        ) -> Result<(), AiCommandError> {
            assert_eq!(draft.client_request_id, "request");
            self.stage("sources");
            Ok(())
        }
        async fn prepare(
            &self,
            _draft: PaperAnalysisStartDraft,
        ) -> Result<PreparedWorkflow, AiCommandError> {
            self.stage("prepare");
            tokio::task::yield_now().await;
            if self.branch == Branch::After {
                self.signal.cancel();
            }
            Ok(PreparedWorkflow {
                workflow: workflow(),
                overview_draft: DoclingPaperOverviewDraft {
                    paper_id: "paper".into(),
                    attachment_ids: vec![],
                    context_snapshot_id: Some("snapshot".into()),
                    overview_profile_selection_revision: 1,
                    paper_template_ref: None,
                    model_selection: None,
                    image_assets: vec![],
                    shared_conversation_id: Some(uuid::Uuid::nil()),
                    shared_request_id: Some(uuid::Uuid::nil()),
                },
            })
        }
        async fn overview(
            &self,
            draft: DoclingPaperOverviewDraft,
            _cancellation: CancellationSignal,
        ) -> Result<String, AiCommandError> {
            assert_eq!(draft.context_snapshot_id.as_deref(), Some("snapshot"));
            self.stage("overview");
            match self.branch {
                Branch::CreateUnknown => Err(AiError::RunCreationUnknown {
                    run_id: uuid::Uuid::nil(),
                    phase: "test",
                    safe_cause: "test",
                    prior_code: None,
                    sqlite_code: None,
                    sqlite_extended_code: None,
                }
                .into()),
                Branch::TerminalUnknown => Err(AiError::RunTerminalUnknown {
                    run_id: "run".into(),
                    phase: "test",
                    safe_cause: "test",
                    prior_code: None,
                    sqlite_code: None,
                    sqlite_extended_code: None,
                }
                .into()),
                Branch::Failure => Err(AiError::Provider("invalid_response").into()),
                _ => Ok("run".into()),
            }
        }
        async fn complete(
            &self,
            id: String,
            run: String,
        ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
            assert_eq!(id, "workflow");
            assert_eq!(run, "run");
            self.stage("complete");
            Ok(workflow())
        }
        async fn failure(
            &self,
            id: String,
            code: String,
            _message: String,
        ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
            assert_eq!(id, "workflow");
            assert_eq!(
                code,
                if self.branch == Branch::After {
                    "canceled"
                } else {
                    "invalid_response"
                }
            );
            self.stage("failure");
            Ok(workflow())
        }
    }
    #[tokio::test]
    async fn cancellation_unknown_outcome_and_completion_keep_the_original_request_and_lease() {
        for branch in [
            Branch::Before,
            Branch::After,
            Branch::CreateUnknown,
            Branch::TerminalUnknown,
            Branch::Failure,
            Branch::Success,
        ] {
            let manager = PaperAnalysisWorkflowManager::default();
            let lease = manager.register_for_paper("request", "paper").unwrap();
            let signal = lease.signal();
            if branch == Branch::Before {
                signal.cancel();
            }
            let host = Host {
                branch,
                signal: signal.clone(),
                manager: manager.clone(),
                events: Mutex::new(vec![]),
            };
            let draft = PaperAnalysisStartDraft {
                regenerate: false,
                client_request_id: "request".into(),
                paper_id: "paper".into(),
                attachment_ids: vec![],
                overview_profile_selection_revision: 1,
                paper_template_ref: None,
                model_selection: None,
                image_assets: vec![],
                skill_source_ids: vec![],
                resource_inputs: vec![],
            };
            let result = start_workflow(&host, draft, signal).await;
            let expected: &[&str] = match branch {
                Branch::Before => &[],
                Branch::After => &["sources", "prepare", "failure"],
                Branch::CreateUnknown | Branch::TerminalUnknown => {
                    &["sources", "prepare", "overview"]
                }
                Branch::Failure => &["sources", "prepare", "overview", "failure"],
                Branch::Success => &["sources", "prepare", "overview", "complete"],
            };
            assert_eq!(*host.events.lock().unwrap(), expected, "{branch:?}");
            match branch {
                Branch::Before => assert_eq!(result.unwrap_err().code(), "canceled"),
                Branch::CreateUnknown => {
                    assert_eq!(result.unwrap_err().code(), "data_operation_outcome_unknown")
                }
                Branch::TerminalUnknown => assert_eq!(
                    result.unwrap_err().code(),
                    "ai_run_terminal_outcome_unknown"
                ),
                _ => {
                    assert_eq!(result.unwrap().id, "workflow");
                }
            }
            assert_eq!(
                manager.active_request("paper").unwrap().as_deref(),
                Some("request")
            );
            drop(lease);
            assert!(manager.active_request("paper").unwrap().is_none());
        }
    }
}
