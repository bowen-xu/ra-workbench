//! Reading-note discovery shares pagination and storage with Note Page recovery.
use super::note_submission::{
    Error,
    recovery::{self, Candidate, SelectedRequest},
    source::Resource,
};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub reading_note_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub reading_note_id: Uuid,
    pub items: Vec<Candidate>,
    pub next_cursor: Option<Uuid>,
}
pub(crate) async fn list(store: &AppStore, request: Request) -> Result<Page, Error> {
    let candidates = recovery::list_selected(
        store,
        SelectedRequest {
            conversation_id: request.conversation_id,
            resource: Resource::Reading(request.reading_note_id),
            after: request.after,
            limit: request.limit,
        },
    )
    .await?;
    Ok(Page {
        schema_version: 1,
        conversation_id: request.conversation_id,
        reading_note_id: request.reading_note_id,
        items: candidates.items,
        next_cursor: candidates.next_cursor,
    })
}
#[tauri::command]
pub(crate) async fn agent_list_unstarted_reading_note_submissions(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Page, Error> {
    list(&store, request).await
}
