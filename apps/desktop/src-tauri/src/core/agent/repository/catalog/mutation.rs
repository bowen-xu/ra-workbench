//! Safe receipt translation shared by concrete catalog mutations.
use crate::data_interface::{
    SqlImmediateDiagnostic, SqlImmediateFailure, SqlImmediateReceipt, SqlImmediateStatus,
};
use research_agent_core::conversation::{
    CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic, CatalogMutationError,
    CatalogMutationProof, CatalogMutationStatus, ConversationId,
};
use uuid::Uuid;

pub(super) fn proof(id: ConversationId, receipt: SqlImmediateReceipt) -> CatalogMutationProof {
    CatalogMutationProof {
        operation_id: receipt.operation_id,
        conversation_id: id,
        epoch: receipt.epoch,
        sql_fence: receipt.sql_fence.position,
        replayed: receipt.replayed,
    }
}
pub(super) fn worker_failure(
    operation_id: Uuid,
    writing: bool,
    phase: &'static str,
    safe_cause: &'static str,
) -> CatalogMutationError {
    CatalogDataFailure {
        kind: if writing {
            CatalogDataFailureKind::Unknown
        } else {
            CatalogDataFailureKind::Unavailable
        },
        diagnostic: Box::new(CatalogMutationDiagnostic {
            operation_id,
            epoch: None,
            sql_fence: None,
            phase,
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause,
        }),
    }
    .into()
}
pub(super) fn mutation_error(error: SqlImmediateFailure) -> CatalogMutationError {
    use CatalogDataFailureKind as K;
    let (kind, diagnostic) = match error {
        SqlImmediateFailure::Domain(error) => return CatalogMutationError::Domain(error),
        SqlImmediateFailure::Invalid(value) => (K::Invalid, value),
        SqlImmediateFailure::Unmanaged(value) => (K::Unmanaged, value),
        SqlImmediateFailure::Conflict(value) => (K::Conflict, value),
        SqlImmediateFailure::Unknown(value) => (K::Unknown, value),
        SqlImmediateFailure::RecoveryRequired(value) => (K::RecoveryRequired, value),
        SqlImmediateFailure::Unavailable(value) => (K::Unavailable, value),
        SqlImmediateFailure::Corrupt(value) => (K::Corrupt, value),
    };
    let SqlImmediateDiagnostic {
        operation_id,
        epoch,
        sql_fence,
        phase,
        sqlite_code,
        sqlite_extended_code,
        safe_cause,
    } = *diagnostic;
    CatalogDataFailure {
        kind,
        diagnostic: Box::new(CatalogMutationDiagnostic {
            operation_id,
            epoch,
            sql_fence,
            phase,
            sqlite_code,
            sqlite_extended_code,
            safe_cause,
        }),
    }
    .into()
}

pub(super) fn status(id: ConversationId, value: SqlImmediateStatus) -> CatalogMutationStatus {
    match value {
        SqlImmediateStatus::Committed(receipt) => {
            CatalogMutationStatus::Committed(proof(id, receipt))
        }
        SqlImmediateStatus::NoSqlReceipt {
            epoch,
            sql_fence,
            closed,
        } => CatalogMutationStatus::NoSqlReceipt {
            epoch,
            sql_fence: sql_fence.position,
            closed,
        },
        SqlImmediateStatus::Unconfirmed {
            epoch,
            sql_fence,
            blocked,
            closed,
            reason,
        } => CatalogMutationStatus::Unconfirmed {
            epoch,
            sql_fence: sql_fence.position,
            blocked,
            closed,
            reason,
        },
    }
}
