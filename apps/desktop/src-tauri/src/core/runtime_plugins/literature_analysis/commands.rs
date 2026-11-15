//! Original history IPC selects the explicitly composed source.
use super::{error::AnalysisError, failure::Error, manager::PaperAnalysisWorkflowManager};
use crate::{
    ai::error::{AiCommandError, AiError},
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_history::{
    HistoryRequestV1, contracts::PaperAnalysisWorkflow,
};
use serde_json::Value;
use tauri::State;
pub(crate) mod start;
pub(crate) use start::start_paper_analysis_workflow;

pub(crate) fn original_error(error: AnalysisError) -> AiError {
    match error {
        AnalysisError::Database(error) => error.into(),
        AnalysisError::InvalidInput(message) => AiError::InvalidInput(message),
        AnalysisError::TemporaryUnavailable => AiError::TemporaryUnavailable,
        AnalysisError::Core(error) => error.into_ai_error(),
    }
}

pub(crate) async fn read_selected(
    store: AppStore,
    manager: PaperAnalysisWorkflowManager,
    request: HistoryRequestV1,
) -> Result<Option<PaperAnalysisWorkflow>, Value> {
    let mut workflow = if let Some(binding) = store.native_note_trace().cloned() {
        binding
            .service
            .read_analysis_history(store, binding.package_key, request.clone())
            .await?
    } else {
        let read = request.clone();
        tokio::task::spawn_blocking(move || {
            let source = store.require_paper_analysis_source()?;
            let db = store.connection()?;
            match read {
                HistoryRequestV1::Get { workflow_id } => {
                    (source.workflow)(&db, &workflow_id).map(Some)
                }
                HistoryRequestV1::Latest { paper_id } => (source.latest)(&db, &paper_id),
            }
            .map_err(|error| error.into_ai_error())
        })
        .await
        .map_err(|_| super::failure::original(AiError::TemporaryUnavailable))?
        .map_err(super::failure::original)?
    };
    if let Some(value) = &mut workflow {
        let paper = match &request {
            HistoryRequestV1::Get { .. } => &value.paper_id,
            HistoryRequestV1::Latest { paper_id } => paper_id,
        };
        value.active_client_request_id = manager
            .active_request(paper)
            .map_err(|error| super::failure::original(original_error(error)))?;
    }
    Ok(workflow)
}

#[tauri::command]
pub(crate) async fn get_paper_analysis_workflow(
    store: State<'_, AppStore>,
    manager: State<'_, PaperAnalysisWorkflowManager>,
    workflow_id: String,
) -> Result<PaperAnalysisWorkflow, Value> {
    let request = HistoryRequestV1::Get { workflow_id };
    read_selected(
        store.inner().clone(),
        manager.inner().clone(),
        request.clone(),
    )
    .await?
    .ok_or_else(|| Error::mismatch().into_wire(&request, None))
}

#[tauri::command]
pub(crate) async fn get_latest_paper_analysis_workflow(
    store: State<'_, AppStore>,
    manager: State<'_, PaperAnalysisWorkflowManager>,
    paper_id: String,
) -> Result<Option<PaperAnalysisWorkflow>, Value> {
    read_selected(
        store.inner().clone(),
        manager.inner().clone(),
        HistoryRequestV1::Latest { paper_id },
    )
    .await
}

#[tauri::command]
pub(crate) fn cancel_paper_analysis_workflow(
    manager: State<'_, PaperAnalysisWorkflowManager>,
    client_request_id: String,
) -> Result<bool, AiCommandError> {
    manager
        .cancel(&client_request_id)
        .map_err(original_error)
        .map_err(Into::into)
}
