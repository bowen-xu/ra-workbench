//! Read-only branch provenance within the original host binding.
use super::{
    catalog_commands::{BindingDto, CatalogCommandError, SummaryDto},
    repository::SqliteConversationCatalog,
};
use research_agent_core::conversation::{
    ConversationForkOrigin, ConversationForkReader, ConversationId,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OriginRequest {
    pub conversation_id: Uuid,
    pub binding: BindingDto,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginDto {
    pub parent: SummaryDto,
    pub through_turn_id: Uuid,
    pub parent_end_seq: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub origin: Option<OriginDto>,
}
impl TryFrom<ConversationForkOrigin> for OriginDto {
    type Error = CatalogCommandError;
    fn try_from(value: ConversationForkOrigin) -> Result<Self, Self::Error> {
        if value.parent.id.as_uuid().is_nil()
            || value.through_turn_id.is_nil()
            || value.parent_end_seq == 0
            || value.parent_end_seq > value.parent.durable_count
        {
            return Err(CatalogCommandError::Corrupt);
        }
        Ok(Self {
            parent: SummaryDto::try_from(value.parent)?,
            through_turn_id: value.through_turn_id,
            parent_end_seq: value.parent_end_seq,
        })
    }
}
pub(crate) async fn execute(
    reader: &dyn ConversationForkReader,
    request: OriginRequest,
) -> Result<OriginResponse, CatalogCommandError> {
    if request.conversation_id.is_nil() {
        return Err(CatalogCommandError::InvalidInput);
    }
    let child = ConversationId::from_uuid(request.conversation_id);
    let origin = reader.read_origin(child, request.binding.decode()?).await?;
    if origin
        .as_ref()
        .is_some_and(|value| value.parent.id == child)
    {
        return Err(CatalogCommandError::Corrupt);
    }
    Ok(OriginResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        origin: origin.map(OriginDto::try_from).transpose()?,
    })
}
#[tauri::command]
pub async fn agent_read_conversation_fork_origin(
    catalog: State<'_, SqliteConversationCatalog>,
    request: OriginRequest,
) -> Result<OriginResponse, CatalogCommandError> {
    execute(catalog.inner(), request).await
}

#[cfg(test)]
#[path = "fork_origin_commands/tests.rs"]
mod tests;
