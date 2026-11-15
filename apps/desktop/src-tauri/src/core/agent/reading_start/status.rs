//! Read the historical admission, without retrying the original worker.
use super::{Error, StartRequest};
use crate::{
    data_interface::{PaperStartInspection, SqlImmediateDiagnostic},
    storage::AppStore,
};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Status {
    Started {
        result: super::super::submission_response::StartResponse,
    },
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    schema_version: u8,
    intent: StartRequest,
    status: Status,
}

#[tauri::command]
pub(crate) async fn agent_read_paper_start_status(
    store: State<'_, AppStore>,
    request: StartRequest,
    expected_epoch: Option<uuid::Uuid>,
) -> Result<Response, Error> {
    read_with_epoch(store.inner(), request, expected_epoch).await
}
pub(crate) async fn read_with_epoch(
    store: &AppStore,
    request: StartRequest,
    expected_epoch: Option<uuid::Uuid>,
) -> Result<Response, Error> {
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.context_snapshot_id.is_nil()
        || request.paper_id.is_empty()
        || request.paper_context_snapshot_id.is_empty()
        || request.model.provider_id.trim().is_empty()
        || request.model.model_id.trim().is_empty()
    {
        return Err(Error::ConfirmationMismatch);
    }
    let database = store.shared_agent_connection();
    let intent = request.clone();
    let observed = tokio::task::spawn_blocking(move || {
        if let Some(epoch) = expected_epoch {
            database
                .check_catalog_epoch(intent.request_id, epoch, "paper-start-status-epoch")
                .map_err(|error| {
                    Error::Preparation(super::reading_submission::storage_error(error))
                })?;
        }
        Ok::<_, Error>(database.inspect_paper_start(&intent))
    })
    .await
    .map_err(|_| Error::RecoveryRequired)??;
    let status = match observed {
        PaperStartInspection::Unconfirmed(detail) => Status::Unconfirmed { detail },
        PaperStartInspection::Started { turn_id, outcome } => Status::Started {
            result: super::super::submission_response::StartResponse {
                schema_version: 1,
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                turn_id,
                run_id: None,
                already_started: true,
                outcome,
            },
        },
    };
    Ok(Response {
        schema_version: 1,
        intent: request,
        status,
    })
}
