//! Native and preview creation share a proof-only execution boundary.
pub use super::mutation::{
    CatalogMutationCommandError as CreateBoundCommandError,
    CatalogMutationProofDto as CreateBoundProofDto,
    CatalogMutationStatusResponse as CreateBoundStatusResponse,
};
use super::{BindingDto, SqliteConversationCatalog};
use research_agent_core::conversation::{
    ConversationCatalog, ConversationId, CreateBoundConversation,
};
use serde::Deserialize;
use uuid::Uuid;
const FAMILY: &str = "agent.conversation.create_bound";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBoundRequest {
    pub operation_id: Uuid,
    pub id: Uuid,
    pub binding: BindingDto,
}
impl CreateBoundRequest {
    fn decode(self) -> Result<CreateBoundConversation, CreateBoundCommandError> {
        let command = CreateBoundConversation {
            operation_id: self.operation_id,
            id: ConversationId::from_uuid(self.id),
            binding: self
                .binding
                .decode()
                .map_err(CreateBoundCommandError::Domain)?,
        };
        command
            .validate()
            .map_err(|error| CreateBoundCommandError::Domain(error.into()))?;
        Ok(command)
    }
}
pub(crate) async fn execute_create_bound(
    catalog: &SqliteConversationCatalog,
    request: CreateBoundRequest,
) -> Result<CreateBoundProofDto, CreateBoundCommandError> {
    let proof = catalog.create_bound(request.decode()?).await?;
    Ok(CreateBoundProofDto::new(proof, FAMILY))
}
pub(crate) async fn execute_create_bound_status(
    catalog: &SqliteConversationCatalog,
    request: CreateBoundRequest,
) -> Result<CreateBoundStatusResponse, CreateBoundCommandError> {
    let command = request.decode()?;
    let operation = command.operation_id;
    let id = *command.id.as_uuid();
    let status = catalog.create_bound_status(command).await?;
    Ok(CreateBoundStatusResponse::new(
        operation, id, status, FAMILY,
    ))
}

#[cfg(test)]
#[path = "creation/tests.rs"]
mod tests;
