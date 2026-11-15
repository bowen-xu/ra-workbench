//! Explicit historical branching. No runtime owner or model is opened here.
pub use super::catalog_commands::{
    CatalogMutationCommandError as ForkCommandError,
    CatalogMutationStatusResponse as ForkStatusResponse,
};
use super::{
    catalog_commands::{BindingDto, CatalogCommandError, SummaryDto},
    repository::SqliteConversationCatalog,
};
use research_agent_core::conversation::{
    ConversationFork, ConversationForkRequest, ConversationForkStore, ConversationId,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkRequest {
    pub child_id: Uuid,
    pub parent_id: Uuid,
    pub through_turn_id: Uuid,
    pub binding: BindingDto,
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
}
impl ForkRequest {
    fn decode(self) -> Result<ConversationForkRequest, CatalogCommandError> {
        if self.child_id.is_nil()
            || self.parent_id.is_nil()
            || self.through_turn_id.is_nil()
            || self.child_id == self.parent_id
            || self.expected_epoch.is_some_and(|epoch| epoch.is_nil())
        {
            return Err(CatalogCommandError::InvalidInput);
        }
        Ok(ConversationForkRequest {
            child_id: ConversationId::from_uuid(self.child_id),
            parent_id: ConversationId::from_uuid(self.parent_id),
            through_turn_id: self.through_turn_id,
            binding: self.binding.decode()?,
        })
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkResponse {
    pub schema_version: u8,
    pub conversation: SummaryDto,
    pub parent_id: Uuid,
    pub through_turn_id: Uuid,
    pub parent_end_seq: u64,
}
impl TryFrom<ConversationFork> for ForkResponse {
    type Error = CatalogCommandError;
    fn try_from(value: ConversationFork) -> Result<Self, Self::Error> {
        if value.conversation.id.as_uuid().is_nil()
            || value.parent_id.as_uuid().is_nil()
            || value.through_turn_id.is_nil()
            || value.conversation.id == value.parent_id
            || value.parent_end_seq == 0
            || value.parent_end_seq > value.conversation.durable_count
        {
            return Err(CatalogCommandError::Corrupt);
        }
        Ok(Self {
            schema_version: 1,
            conversation: SummaryDto::try_from(value.conversation)?,
            parent_id: *value.parent_id.as_uuid(),
            through_turn_id: value.through_turn_id,
            parent_end_seq: value.parent_end_seq,
        })
    }
}
pub(crate) async fn execute(
    catalog: &dyn ConversationForkStore,
    request: ForkRequest,
) -> Result<ForkResponse, ForkCommandError> {
    let expected_epoch = request.expected_epoch;
    let request = request.decode().map_err(ForkCommandError::Domain)?;
    let result = if let Some(epoch) = expected_epoch {
        catalog.fork_in_epoch(request.clone(), epoch).await?
    } else {
        catalog.fork(request.clone()).await?
    };
    if result.conversation.id != request.child_id
        || result.parent_id != request.parent_id
        || result.through_turn_id != request.through_turn_id
    {
        return Err(ForkCommandError::Domain(CatalogCommandError::Corrupt));
    }
    ForkResponse::try_from(result).map_err(ForkCommandError::Domain)
}
pub(crate) async fn execute_status(
    catalog: &dyn ConversationForkStore,
    request: ForkRequest,
) -> Result<ForkStatusResponse, ForkCommandError> {
    let request = request.decode().map_err(ForkCommandError::Domain)?;
    let child = *request.child_id.as_uuid();
    let status = catalog.fork_status(request).await?;
    Ok(ForkStatusResponse::new(
        child,
        child,
        status,
        "agent.conversation.fork",
    ))
}
#[tauri::command]
pub async fn agent_fork_conversation(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ForkRequest,
) -> Result<ForkResponse, ForkCommandError> {
    execute(catalog.inner(), request).await
}
#[tauri::command]
pub async fn agent_read_conversation_fork_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ForkRequest,
) -> Result<ForkStatusResponse, ForkCommandError> {
    execute_status(catalog.inner(), request).await
}

#[cfg(test)]
#[path = "fork_commands/tests.rs"]
mod tests;
