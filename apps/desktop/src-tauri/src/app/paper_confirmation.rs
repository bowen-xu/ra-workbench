//! App selects the explicit Native binding or the same original static composition.

use crate::{
    ai::{
        confirmation_support::*,
        error::{AiCommandError, AiError},
    },
    runtime_plugins::literature_paper_confirmation::ConfirmationResult,
    storage::AppStore,
};
use research_workbench_native_sdk::paper_confirmation::PaperConfirmationRequestV1 as Request;

use tauri::State;

pub(crate) async fn execute_selected(
    store: &AppStore,
    request: Request,
) -> Result<ConfirmationResult, AiError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .paper_confirmation(store.clone(), binding.package_key, request)
            .await;
    }

    {
        let _ = request;
        Err(AiError::CapabilityUnavailable(
            "literature_library.paper_confirmation",
        ))
    }
}
fn overview(result: ConfirmationResult) -> Result<AiOverviewConfirmationResult, AiError> {
    match result {
        ConfirmationResult::Overview(value) => Ok(*value),
        _ => Err(AiError::TemporaryUnavailable),
    }
}
fn paper(result: ConfirmationResult) -> Result<AiPaperReviewConfirmationResult, AiError> {
    match result {
        ConfirmationResult::Paper(value) => Ok(*value),
        _ => Err(AiError::TemporaryUnavailable),
    }
}

#[tauri::command]
pub(crate) async fn confirm_ai_overview_fields(
    store: State<'_, AppStore>,
    draft: AiOverviewFieldConfirmationDraft,
) -> Result<AiOverviewConfirmationResult, AiCommandError> {
    overview(execute_selected(&store, Request::ConfirmOverview { draft }).await?)
        .map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn undo_ai_overview_confirmation(
    store: State<'_, AppStore>,
    draft: AiOverviewConfirmationUndoDraft,
) -> Result<AiOverviewConfirmationResult, AiCommandError> {
    overview(execute_selected(&store, Request::UndoOverview { draft }).await?).map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn confirm_ai_paper_review(
    store: State<'_, AppStore>,
    draft: AiPaperReviewConfirmationDraft,
) -> Result<AiPaperReviewConfirmationResult, AiCommandError> {
    paper(execute_selected(&store, Request::ConfirmPaper { draft }).await?).map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn undo_ai_paper_review_confirmation(
    store: State<'_, AppStore>,
    draft: AiPaperReviewConfirmationUndoDraft,
) -> Result<AiPaperReviewConfirmationResult, AiCommandError> {
    paper(execute_selected(&store, Request::UndoPaper { draft }).await?).map_err(Into::into)
}
