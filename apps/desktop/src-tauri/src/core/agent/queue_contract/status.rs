//! One status DTO for each host's original user edit; it never reads current Q.
use super::{CommandError, QueueState};
use crate::{
    agent::catalog_commands::CatalogMutationProofDto,
    data_interface::{
        QUEUE_EDIT_FAMILY, QueueEditIntent, QueueEditStatus, SqlImmediateDiagnostic,
        SqlImmediateReceipt,
    },
};
use research_agent_core::conversation::{CatalogMutationProof, ConversationId};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditStatusResponse {
    schema_version: u8,
    intent: QueueEditIntent,
    status: EditStatus,
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum EditStatus {
    Committed {
        queue: QueueState,
        proof: CatalogMutationProofDto,
    },
    Deleted {
        proof: CatalogMutationProofDto,
        detail: Box<SqlImmediateDiagnostic>,
    },
    NotFound,
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}
fn proof(intent: &QueueEditIntent, receipt: SqlImmediateReceipt) -> CatalogMutationProofDto {
    CatalogMutationProofDto::new(
        CatalogMutationProof {
            operation_id: receipt.operation_id,
            conversation_id: ConversationId::from_uuid(intent.conversation_id),
            epoch: receipt.epoch,
            sql_fence: receipt.sql_fence.position,
            replayed: receipt.replayed,
        },
        QUEUE_EDIT_FAMILY,
    )
}
pub(crate) fn edit_status(
    intent: QueueEditIntent,
    status: QueueEditStatus,
) -> Result<EditStatusResponse, CommandError> {
    let status = match status {
        QueueEditStatus::Committed { queue, receipt } => EditStatus::Committed {
            queue: super::state(queue)?,
            proof: proof(&intent, receipt),
        },
        QueueEditStatus::Deleted { receipt, detail } => EditStatus::Deleted {
            proof: proof(&intent, receipt),
            detail,
        },
        QueueEditStatus::NotFound => EditStatus::NotFound,
        QueueEditStatus::Unconfirmed(detail) => EditStatus::Unconfirmed { detail },
    };
    Ok(EditStatusResponse {
        schema_version: 1,
        intent,
        status,
    })
}
#[cfg(test)]
#[path = "status/tests.rs"]
mod tests;
