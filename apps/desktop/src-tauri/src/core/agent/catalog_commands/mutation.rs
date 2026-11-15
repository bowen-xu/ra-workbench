//! Shared immutable catalog proof and safe storage DTOs.
use super::CatalogCommandError;
use research_agent_core::conversation::{
    CatalogDataFailure, CatalogMutationError, CatalogMutationProof, CatalogMutationStatus,
};
use serde::{Serialize, ser::SerializeStruct};
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMutationProofDto {
    schema_version: u8,
    operation_id: Uuid,
    conversation_id: Uuid,
    family: &'static str,
    epoch: Uuid,
    sql_fence: String,
    replayed: bool,
    result: (),
}
impl CatalogMutationProofDto {
    pub(crate) fn new(proof: CatalogMutationProof, family: &'static str) -> Self {
        Self {
            schema_version: 1,
            operation_id: proof.operation_id,
            conversation_id: *proof.conversation_id.as_uuid(),
            family,
            epoch: proof.epoch,
            sql_fence: proof.sql_fence.to_string(),
            replayed: proof.replayed,
            result: (),
        }
    }
}
#[derive(Debug)]
pub enum CatalogMutationCommandError {
    Domain(CatalogCommandError),
    Data(CatalogDataFailure),
}
impl From<CatalogMutationError> for CatalogMutationCommandError {
    fn from(error: CatalogMutationError) -> Self {
        match error {
            CatalogMutationError::Domain(error) => Self::Domain(error.into()),
            CatalogMutationError::Data(error) => Self::Data(error),
        }
    }
}
impl Serialize for CatalogMutationCommandError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Domain(error) => error.serialize(serializer),
            Self::Data(error) => {
                let mut value = serializer.serialize_struct("CatalogMutationCommandError", 2)?;
                value.serialize_field("code", "storage")?;
                value.serialize_field("reason", error)?;
                value.end()
            }
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMutationStatusResponse {
    schema_version: u8,
    operation_id: Uuid,
    conversation_id: Uuid,
    status: CatalogMutationStatusDto,
}
#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum CatalogMutationStatusDto {
    Committed {
        proof: CatalogMutationProofDto,
    },
    NoSqlReceipt {
        epoch: Uuid,
        sql_fence: String,
        closed: bool,
    },
    Unconfirmed {
        epoch: Uuid,
        sql_fence: String,
        blocked: bool,
        closed: bool,
        reason: &'static str,
    },
}
impl CatalogMutationStatusDto {
    pub(super) fn new(status: CatalogMutationStatus, family: &'static str) -> Self {
        match status {
            CatalogMutationStatus::Committed(proof) => Self::Committed {
                proof: CatalogMutationProofDto::new(proof, family),
            },
            CatalogMutationStatus::NoSqlReceipt {
                epoch,
                sql_fence,
                closed,
            } => Self::NoSqlReceipt {
                epoch,
                sql_fence: sql_fence.to_string(),
                closed,
            },
            CatalogMutationStatus::Unconfirmed {
                epoch,
                sql_fence,
                blocked,
                closed,
                reason,
            } => Self::Unconfirmed {
                epoch,
                sql_fence: sql_fence.to_string(),
                blocked,
                closed,
                reason,
            },
        }
    }
}

impl CatalogMutationStatusResponse {
    pub(crate) fn new(
        operation_id: Uuid,
        conversation_id: Uuid,
        status: CatalogMutationStatus,
        family: &'static str,
    ) -> Self {
        Self {
            schema_version: 1,
            operation_id,
            conversation_id,
            status: CatalogMutationStatusDto::new(status, family),
        }
    }
}
