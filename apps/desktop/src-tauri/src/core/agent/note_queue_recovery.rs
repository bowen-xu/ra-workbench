//! Recover the durable gap after queue admission without automatically starting a model.
use super::{
    note_queue::Scope,
    note_submission::{self, recovery},
};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub after: Option<Uuid>,
    pub limit: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub items: Vec<recovery::Candidate>,
    pub next_cursor: Option<Uuid>,
}
pub(crate) async fn list(
    store: &AppStore,
    request: Request,
) -> Result<Page, note_submission::Error> {
    let candidates = recovery::list_queue_selected(
        store,
        recovery::SelectedRequest {
            conversation_id: request.conversation_id,
            resource: request.scope.resource(),
            after: request.after,
            limit: request.limit,
        },
    )
    .await?;
    Ok(Page {
        schema_version: 1,
        conversation_id: request.conversation_id,
        scope: request.scope,
        items: candidates.items,
        next_cursor: candidates.next_cursor,
    })
}
#[tauri::command]
pub(crate) async fn agent_list_unstarted_note_queue_admissions(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Page, note_submission::Error> {
    list(&store, request).await
}
