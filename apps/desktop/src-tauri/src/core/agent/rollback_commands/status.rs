//! Receipt inspection is read-only. Committed registration is not live authority
//! and does not prove that the separate journal rollback executed.
use super::{Error, SqliteConversationCatalog, authorization_error};
use crate::{
    agent::catalog_commands::BindingDto,
    data_interface::{AuthorizeRollback, SqlImmediateStatus},
};
use research_agent_core::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AuthorizationStatusRequest {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub binding: BindingDto,
}
impl AuthorizationStatusRequest {
    fn decode(self) -> Result<AuthorizeRollback, Error> {
        if self.schema_version != 1 || self.conversation_id.is_nil() || self.request_id.is_nil() {
            return Err(Error::InvalidInput);
        }
        Ok(AuthorizeRollback {
            operation_id: self.request_id,
            conversation_id: ConversationId::from_uuid(self.conversation_id),
            binding: self.binding.decode().map_err(|_| Error::InvalidInput)?,
        })
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizationStatusResponse {
    schema_version: u8,
    conversation_id: Uuid,
    request_id: Uuid,
    status: AuthorizationStatus,
}
#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum AuthorizationStatus {
    Committed {
        epoch: Uuid,
        sql_fence: String,
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
impl From<SqlImmediateStatus> for AuthorizationStatus {
    fn from(status: SqlImmediateStatus) -> Self {
        match status {
            SqlImmediateStatus::Committed(receipt) => Self::Committed {
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position.to_string(),
            },
            SqlImmediateStatus::NoSqlReceipt {
                epoch,
                sql_fence,
                closed,
            } => Self::NoSqlReceipt {
                epoch,
                sql_fence: sql_fence.position.to_string(),
                closed,
            },
            SqlImmediateStatus::Unconfirmed {
                epoch,
                sql_fence,
                blocked,
                closed,
                reason,
            } => Self::Unconfirmed {
                epoch,
                sql_fence: sql_fence.position.to_string(),
                blocked,
                closed,
                reason,
            },
        }
    }
}
pub(crate) async fn execute_status(
    catalog: &SqliteConversationCatalog,
    request: AuthorizationStatusRequest,
) -> Result<AuthorizationStatusResponse, Error> {
    let command = request.decode()?;
    let conversation_id = *command.conversation_id.as_uuid();
    let request_id = command.operation_id;
    let status = catalog
        .read_rollback_authorization_status(command)
        .await
        .map_err(authorization_error)?;
    Ok(AuthorizationStatusResponse {
        schema_version: 1,
        conversation_id,
        request_id,
        status: status.into(),
    })
}
#[cfg(test)]
#[path = "status/tests.rs"]
mod tests;
