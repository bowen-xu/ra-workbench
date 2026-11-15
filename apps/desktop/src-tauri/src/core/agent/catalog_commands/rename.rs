//! Both native edges use the same proof-only rename and read-only status use cases.
use super::SqliteConversationCatalog;
#[cfg(test)]
use super::mutation::CatalogMutationStatusDto as RenameStatusDto;
pub use super::mutation::{
    CatalogMutationCommandError as RenameCommandError, CatalogMutationProofDto as RenameProofDto,
    CatalogMutationStatusResponse as RenameStatusResponse,
};
use research_agent_core::conversation::{ConversationCatalog, ConversationId, RenameConversation};
use serde::Deserialize;
const FAMILY: &str = "agent.conversation.rename";
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameRequest {
    pub operation_id: Uuid,
    pub id: Uuid,
    pub expected_title: String,
    pub title: String,
}
impl RenameRequest {
    fn decode(self) -> Result<RenameConversation, RenameCommandError> {
        let command = RenameConversation {
            operation_id: self.operation_id,
            id: ConversationId::from_uuid(self.id),
            expected_title: self.expected_title,
            title: self.title,
        };
        command
            .validate()
            .map_err(|error| RenameCommandError::Domain(error.into()))?;
        Ok(command)
    }
}
pub(crate) async fn execute_rename(
    catalog: &SqliteConversationCatalog,
    request: RenameRequest,
) -> Result<RenameProofDto, RenameCommandError> {
    Ok(RenameProofDto::new(
        catalog.rename(request.decode()?).await?,
        FAMILY,
    ))
}
pub(crate) async fn execute_rename_status(
    catalog: &SqliteConversationCatalog,
    request: RenameRequest,
) -> Result<RenameStatusResponse, RenameCommandError> {
    let command = request.decode()?;
    let operation_id = command.operation_id;
    let conversation_id = *command.id.as_uuid();
    let status = catalog.rename_status(command).await?;
    Ok(RenameStatusResponse::new(
        operation_id,
        conversation_id,
        status,
        FAMILY,
    ))
}

#[cfg(test)]
use research_agent_core::conversation::{CatalogDataFailure, RenameProof, RenameStatus};
#[cfg(test)]
#[path = "rename/tests.rs"]
mod tests;
