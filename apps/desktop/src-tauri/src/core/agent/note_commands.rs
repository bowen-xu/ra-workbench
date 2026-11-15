//! Note host disclosure previews; these commands do not start an Agent turn.
use super::{
    model::AgentModelResolver,
    note_submission::{self, Error, PrepareRequest, Receipt},
};
use crate::storage::AppStore;
use research_agent_core::submission::{SubmissionInput, SubmissionRecord};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub page_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: SubmissionInput,
}
impl Preview {
    pub(crate) fn from_record(record: SubmissionRecord, page_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            page_id,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input: record.input,
        }
    }
}

#[tauri::command]
pub(crate) async fn agent_prepare_note_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    let page_id = request.context.page_id;
    Ok(Preview::from_record(
        note_submission::prepare(&store, &models, request).await?,
        page_id,
    ))
}

#[tauri::command]
pub(crate) async fn agent_restore_note_submission(
    store: State<'_, AppStore>,
    request: Receipt,
) -> Result<Preview, Error> {
    Ok(Preview::from_record(
        note_submission::restore(&store, request).await?,
        request.page_id,
    ))
}

#[tauri::command]
pub(crate) async fn agent_list_unstarted_note_submissions(
    store: State<'_, AppStore>,
    request: note_submission::recovery::Request,
) -> Result<note_submission::recovery::Page, Error> {
    note_submission::recovery::list(&store, request).await
}
