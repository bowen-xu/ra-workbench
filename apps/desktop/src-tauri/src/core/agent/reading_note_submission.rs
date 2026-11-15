//! Reading-note disclosure review; no inference or note mutation occurs here.
use super::{
    model::{AgentModelResolver, ModelSelection},
    note_submission::{
        self, Error, SelectedRequest,
        source::{Resource, Selection, SourceReceipt},
    },
    retrieval_context::RetrievalSelection,
};
use crate::{agent::config::ConfigOverride, notes::reading_agent_context, storage::AppStore};
use research_agent_core::submission::{SubmissionInput, SubmissionRecord};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub context: reading_agent_context::Request,
    #[serde(default)]
    pub retrieval: Option<RetrievalSelection>,
    pub model: Option<ModelSelection>,
    #[serde(default)]
    pub session_config: Option<ConfigOverride>,
    #[serde(default)]
    pub turn_config: Option<ConfigOverride>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub reading_note_id: Uuid,
}
impl From<Receipt> for SourceReceipt {
    fn from(receipt: Receipt) -> Self {
        Self {
            conversation_id: receipt.conversation_id,
            request_id: receipt.request_id,
            resource: Resource::Reading(receipt.reading_note_id),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub reading_note_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: SubmissionInput,
}
impl Preview {
    fn from_record(record: SubmissionRecord, reading_note_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            reading_note_id,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input: record.input,
        }
    }
}
pub(crate) async fn prepare(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<SubmissionRecord, Error> {
    note_submission::prepare_selected(
        store,
        models,
        SelectedRequest {
            user_input: request.user_input,
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            text: request.text,
            context: Selection::Reading(request.context),
            retrieval: request.retrieval,
            model: request.model,
            session_config: request.session_config.map(Into::into),
            turn_config: request.turn_config.map(Into::into),
        },
    )
    .await
}
pub(crate) async fn restore(store: &AppStore, receipt: Receipt) -> Result<SubmissionRecord, Error> {
    note_submission::restore_selected(store, receipt.into()).await
}
#[tauri::command]
pub(crate) async fn agent_prepare_reading_note_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    let id = request.context.reading_note_id;
    Ok(Preview::from_record(
        prepare(&store, &models, request).await?,
        id,
    ))
}
#[tauri::command]
pub(crate) async fn agent_restore_reading_note_submission(
    store: State<'_, AppStore>,
    request: Receipt,
) -> Result<Preview, Error> {
    Ok(Preview::from_record(
        restore(&store, request).await?,
        request.reading_note_id,
    ))
}
