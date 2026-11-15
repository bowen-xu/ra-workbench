//! Read the exact preparation fact without authorizing another execution.
use super::{Error, PrepareRequest, Preview, Receipt, storage_error};
use crate::agent::catalog_commands::CatalogMutationProofDto;
use crate::data_interface::{
    PAPER_PREPARE_FAMILY, PaperPrepareIntent, PaperPrepareStatus, SqlImmediateDiagnostic,
};
use crate::storage::AppStore;
use research_agent_core::conversation::{CatalogMutationProof, ConversationId};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Status {
    Committed {
        proof: CatalogMutationProofDto,
        result: Box<Preview>,
    },
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    schema_version: u8,
    intent: PrepareRequest,
    status: Status,
}

pub(super) fn committed(
    request: PrepareRequest,
    record: research_agent_core::submission::SubmissionRecord,
    receipt: crate::data_interface::SqlImmediateReceipt,
) -> Response {
    let result = Preview::from_record(
        record,
        &Receipt {
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            paper_id: request.paper_id.clone(),
            paper_context_snapshot_id: request.paper_context_snapshot_id.clone(),
        },
    );
    let proof = CatalogMutationProofDto::new(
        CatalogMutationProof {
            operation_id: receipt.operation_id,
            conversation_id: ConversationId::from_uuid(request.conversation_id),
            epoch: receipt.epoch,
            sql_fence: receipt.sql_fence.position,
            replayed: receipt.replayed,
        },
        PAPER_PREPARE_FAMILY,
    );
    Response {
        schema_version: 1,
        intent: request,
        status: Status::Committed {
            proof,
            result: Box::new(result),
        },
    }
}

#[tauri::command]
pub(crate) async fn agent_read_paper_submission_prepare_status(
    store: State<'_, AppStore>,
    request: PrepareRequest,
    expected_epoch: Option<uuid::Uuid>,
) -> Result<Response, Error> {
    read_with_epoch(store.inner(), request, expected_epoch).await
}

pub(crate) async fn read_with_epoch(
    store: &AppStore,
    request: PrepareRequest,
    expected_epoch: Option<uuid::Uuid>,
) -> Result<Response, Error> {
    let database = store.shared_agent_connection();
    let intent = PaperPrepareIntent {
        request: request.clone(),
        expected_epoch,
    };
    let status = if let Some(binding) = store.native_note_trace() {
        binding
            .service
            .paper_prepare_status(store.clone(), binding.package_key.clone(), intent)
            .await?
    } else {
        let reading_source = store
            .require_paper_reading_source()
            .map_err(|_| Error::Unavailable)?;
        tokio::task::spawn_blocking(move || database.paper_prepare_status(&intent, reading_source))
            .await
            .map_err(|_| Error::Unavailable)?
            .map_err(storage_error)?
    };
    let status = match status {
        PaperPrepareStatus::Unconfirmed(detail) => Status::Unconfirmed { detail },
        PaperPrepareStatus::Committed { record, receipt } => {
            return Ok(committed(request, *record, *receipt));
        }
    };
    Ok(Response {
        schema_version: 1,
        intent: request,
        status,
    })
}
