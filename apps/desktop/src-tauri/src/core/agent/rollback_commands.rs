//! Host-bound history rollback; the existing task owner holds write admission.
use super::{
    catalog_commands::BindingDto,
    repository::{AuthorizationError, SqliteConversationCatalog},
    service::{AgentServiceError, AgentSessionService, ConversationRollbackRequest},
    session_commands::SessionCommandError,
};
use crate::data_interface::SqlImmediateFailure;
use research_agent_core::conversation::{CatalogError, ConversationBinding, ConversationId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
#[path = "rollback_commands/status.rs"]
mod status;
pub(crate) use status::{AuthorizationStatusRequest, AuthorizationStatusResponse, execute_status};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RollbackRequest {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub binding: BindingDto,
    pub expected_watermark: u64,
    pub expected_last_turn: Uuid,
    pub user_turns: u32,
}
impl RollbackRequest {
    fn decode(self) -> Result<(ConversationRollbackRequest, ConversationBinding), Error> {
        if self.schema_version != 1
            || self.conversation_id.is_nil()
            || self.request_id.is_nil()
            || self.expected_last_turn.is_nil()
            || self.user_turns == 0
            || self.expected_watermark >= 9_007_199_254_740_991
        {
            return Err(Error::InvalidInput);
        }
        let binding = self.binding.decode().map_err(|_| Error::InvalidInput)?;
        Ok((
            ConversationRollbackRequest {
                conversation_id: ConversationId::from_uuid(self.conversation_id),
                request_id: self.request_id,
                expected_watermark: self.expected_watermark,
                expected_last_turn: self.expected_last_turn,
                user_turns: self.user_turns,
            },
            binding,
        ))
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub watermark: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub(crate) enum Error {
    InvalidInput,
    HostUnavailable,
    Conflict,
    Session(SessionCommandError),
    Storage(SqlImmediateFailure),
}
pub(crate) async fn execute(
    catalog: &SqliteConversationCatalog,
    service: &Arc<AgentSessionService>,
    request: RollbackRequest,
) -> Result<RollbackResponse, Error> {
    let (request, binding) = request.decode()?;
    catalog
        .authorize_rollback(request.conversation_id, request.request_id, binding)
        .await
        .map_err(authorization_error)?;
    finish(service, request).await
}
fn authorization_error(error: AuthorizationError) -> Error {
    match error {
        AuthorizationError::Domain(error) => catalog_error(error),
        AuthorizationError::Data(error) => Error::Storage(error),
    }
}
fn catalog_error(error: CatalogError) -> Error {
    match error {
        CatalogError::InvalidInput => Error::InvalidInput,
        CatalogError::NotFound => Error::HostUnavailable,
        CatalogError::Conflict => Error::Conflict,
        _ => Error::Session(SessionCommandError::Unavailable),
    }
}

/// Existing pure-SQL fixtures opt in explicitly, without a managed SQL receipt.
#[cfg(test)]
async fn execute_legacy_for_tests(
    catalog: &SqliteConversationCatalog,
    service: &Arc<AgentSessionService>,
    request: RollbackRequest,
) -> Result<RollbackResponse, Error> {
    let (request, binding) = request.decode()?;
    catalog
        .authorize_rollback_legacy_for_tests(request.conversation_id, request.request_id, binding)
        .await
        .map_err(catalog_error)?;
    finish(service, request).await
}

async fn finish(
    service: &Arc<AgentSessionService>,
    request: ConversationRollbackRequest,
) -> Result<RollbackResponse, Error> {
    let receipt = service
        .rollback_conversation(request)
        .await
        .map_err(|error| match error {
            AgentServiceError::RollbackConflict => Error::Conflict,
            other => Error::Session(other.into()),
        })?;
    Ok(RollbackResponse {
        schema_version: 1,
        conversation_id: *request.conversation_id.as_uuid(),
        request_id: request.request_id,
        watermark: receipt.watermark,
    })
}
#[tauri::command]
pub(crate) async fn agent_rollback_conversation(
    catalog: State<'_, SqliteConversationCatalog>,
    service: State<'_, Arc<AgentSessionService>>,
    request: RollbackRequest,
) -> Result<RollbackResponse, Error> {
    execute(catalog.inner(), service.inner(), request).await
}

#[tauri::command]
pub(crate) async fn agent_read_rollback_authorization_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: AuthorizationStatusRequest,
) -> Result<AuthorizationStatusResponse, Error> {
    execute_status(catalog.inner(), request).await
}

#[cfg(test)]
#[path = "rollback_commands/tests.rs"]
mod tests;
