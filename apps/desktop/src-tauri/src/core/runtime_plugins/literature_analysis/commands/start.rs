//! The original command and task lease are owned by Core in either composition.
use super::*;
use crate::{
    agent::{model::AgentModelResolver, service::AgentSessionService},
    ai::{
        JsonCredentialStore, analysis_support::AnalysisExecution,
        paper_analysis_contracts::PaperAnalysisStartDraft,
    },
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn start_paper_analysis_workflow(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    manager: State<'_, PaperAnalysisWorkflowManager>,
    draft: PaperAnalysisStartDraft,
) -> Result<PaperAnalysisWorkflow, AiCommandError> {
    let execution = AnalysisExecution::new(
        store.inner().clone(),
        credentials.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
    );
    start_selected(
        store.inner().clone(),
        execution,
        manager.inner().clone(),
        draft,
    )
    .await
}

pub(crate) async fn start_selected(
    store: AppStore,
    execution: AnalysisExecution,
    manager: PaperAnalysisWorkflowManager,
    draft: PaperAnalysisStartDraft,
) -> Result<PaperAnalysisWorkflow, AiCommandError> {
    let lease = manager
        .register_for_paper(&draft.client_request_id, &draft.paper_id)
        .map_err(original_error)?;
    tokio::spawn(async move {
        let signal = lease.signal();
        let result = if let Some(binding) = store.native_note_trace().cloned() {
            let host = super::super::execution::NativeAnalysisHost {
                store,
                execution,
                service: binding.service,
                key: binding.package_key,
            };
            Box::pin(crate::ai::analysis_support::start_workflow(
                &host, draft, signal,
            ))
            .await
        } else {
            (store.require_paper_analysis_source()?.start)(store, execution, draft, signal).await
        };
        drop(lease);
        result
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))?
}
