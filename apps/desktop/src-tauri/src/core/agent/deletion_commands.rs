//! Confirmed conversation deletion, coordinated with application-owned runtime admission.
use super::{
    catalog_commands::BindingDto, repository::SqliteConversationCatalog,
    service::AgentSessionService,
};
use research_agent_core::conversation::ConversationDeletionStore;
use research_agent_core::conversation::{
    CatalogError, ConversationDeletionError, ConversationDeletionReceipt,
    ConversationDeletionRequest, ConversationId, ConversationRetention,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteRequest {
    pub schema_version: u8,
    pub request_id: Uuid,
    pub conversation_id: Uuid,
    pub binding: Option<BindingDto>,
    pub expected_title: String,
    pub expected_durable_count: u64,
    #[serde(default)]
    pub force: bool,
    /// Transport-only data precondition; never part of the immutable request hash.
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
}
impl DeleteRequest {
    fn decode(self) -> Result<ConversationDeletionRequest, DeleteError> {
        if self.schema_version != 1 || self.expected_epoch.is_some_and(|epoch| epoch.is_nil()) {
            return Err(DeleteError::InvalidInput);
        }
        let request = ConversationDeletionRequest {
            request_id: self.request_id,
            conversation_id: ConversationId::from_uuid(self.conversation_id),
            binding: self
                .binding
                .map(BindingDto::decode)
                .transpose()
                .map_err(|_| DeleteError::InvalidInput)?,
            expected_title: self.expected_title,
            expected_durable_count: self.expected_durable_count,
            force: self.force,
        };
        request.validate().map_err(|_| DeleteError::InvalidInput)?;
        Ok(request)
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResponse {
    pub schema_version: u8,
    pub request_id: Uuid,
    pub conversation_id: Uuid,
}
impl From<ConversationDeletionReceipt> for DeleteResponse {
    fn from(receipt: ConversationDeletionReceipt) -> Self {
        Self {
            schema_version: 1,
            request_id: receipt.request_id,
            conversation_id: *receipt.conversation_id.as_uuid(),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDeleteResponse {
    pub schema_version: u8,
    pub results: Vec<BatchDeleteItem>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum BatchDeleteItem {
    Deleted {
        #[serde(rename = "requestId")]
        request_id: Uuid,
        #[serde(rename = "conversationId")]
        conversation_id: Uuid,
        #[serde(rename = "cleanupPending")]
        cleanup_pending: bool,
    },
    Failed {
        #[serde(rename = "requestId")]
        request_id: Uuid,
        #[serde(rename = "conversationId")]
        conversation_id: Uuid,
        error: DeleteError,
    },
}

#[tauri::command]
pub async fn agent_delete_conversations_batch(
    service: State<'_, Arc<AgentSessionService>>,
    catalog: State<'_, SqliteConversationCatalog>,
    requests: Vec<DeleteRequest>,
) -> Result<BatchDeleteResponse, DeleteError> {
    execute_batch(service.inner(), catalog.inner(), requests).await
}
pub(crate) async fn execute_batch(
    service: &Arc<AgentSessionService>,
    catalog: &SqliteConversationCatalog,
    requests: Vec<DeleteRequest>,
) -> Result<BatchDeleteResponse, DeleteError> {
    if requests
        .iter()
        .any(|request| request.expected_epoch.is_some())
    {
        return Err(DeleteError::InvalidInput);
    }
    let requests = requests
        .into_iter()
        .map(DeleteRequest::decode)
        .collect::<Result<Vec<_>, _>>()?;
    let identities: Vec<_> = requests
        .iter()
        .map(|request| (request.request_id, *request.conversation_id.as_uuid()))
        .collect();
    let results = service
        .delete_conversations_batch(catalog.clone(), requests)
        .await
        .map_err(DeleteError::from)?;
    let results = identities
        .into_iter()
        .zip(results)
        .map(|((request_id, conversation_id), result)| match result {
            Ok((_, cleanup_pending)) => BatchDeleteItem::Deleted {
                request_id,
                conversation_id,
                cleanup_pending,
            },
            Err(error) => BatchDeleteItem::Failed {
                request_id,
                conversation_id,
                error: DeleteError::from(error),
            },
        })
        .collect();
    Ok(BatchDeleteResponse {
        schema_version: 1,
        results,
    })
}
#[derive(Debug, PartialEq, Eq)]
pub enum DeleteError {
    InvalidInput,
    NotFound,
    Conflict,
    Busy,
    TooLarge,
    Corrupt,
    Unavailable,
    NotArchived,
    BranchHistory,
    ProposalApplications,
    ExecutionWorkspace,
    Data(research_agent_core::conversation::CatalogDataFailure),
}
impl Serialize for DeleteError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Self::Data(cause) = self {
            return super::catalog_commands::CatalogMutationCommandError::Data(cause.clone())
                .serialize(serializer);
        }
        serializer.serialize_str(match self {
            Self::InvalidInput => "invalidInput",
            Self::NotFound => "notFound",
            Self::Conflict => "conflict",
            Self::Busy => "busy",
            Self::TooLarge => "tooLarge",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
            Self::NotArchived => "notArchived",
            Self::BranchHistory => "branchHistory",
            Self::ProposalApplications => "proposalApplications",
            Self::ExecutionWorkspace => "executionWorkspace",
            Self::Data(_) => unreachable!(),
        })
    }
}
impl From<ConversationDeletionError> for DeleteError {
    fn from(error: ConversationDeletionError) -> Self {
        match error {
            ConversationDeletionError::Data(error) => Self::Data(error),
            ConversationDeletionError::Catalog(error) => match error {
                CatalogError::InvalidInput => Self::InvalidInput,
                CatalogError::NotFound => Self::NotFound,
                CatalogError::Conflict => Self::Conflict,
                CatalogError::Busy => Self::Busy,
                CatalogError::TooLarge => Self::TooLarge,
                CatalogError::Corrupt => Self::Corrupt,
                CatalogError::Unavailable => Self::Unavailable,
            },
            ConversationDeletionError::NotArchived => Self::NotArchived,
            ConversationDeletionError::Retained(reason) => match reason {
                ConversationRetention::BranchHistory => Self::BranchHistory,
                ConversationRetention::ProposalApplications => Self::ProposalApplications,
                ConversationRetention::ExecutionWorkspace => Self::ExecutionWorkspace,
            },
        }
    }
}
#[tauri::command]
pub async fn agent_delete_conversation(
    service: State<'_, Arc<AgentSessionService>>,
    catalog: State<'_, SqliteConversationCatalog>,
    request: DeleteRequest,
) -> Result<DeleteResponse, DeleteError> {
    execute(service.inner(), catalog.inner(), request).await
}
pub(crate) async fn execute(
    service: &Arc<AgentSessionService>,
    catalog: &SqliteConversationCatalog,
    request: DeleteRequest,
) -> Result<DeleteResponse, DeleteError> {
    let expected = request.expected_epoch;
    let request = request.decode()?;
    let result = if let Some(epoch) = expected {
        // Reject a foreign environment before reserving or releasing its session.
        catalog
            .check_deletion_epoch(request.request_id, epoch)
            .await
            .map_err(DeleteError::from)?;
        service
            .delete_conversation_in_epoch(Arc::new(catalog.clone()), request, epoch)
            .await
    } else {
        service
            .delete_conversation(Arc::new(catalog.clone()), request)
            .await
    };
    result.map(DeleteResponse::from).map_err(DeleteError::from)
}
pub(crate) async fn execute_status(
    catalog: &dyn ConversationDeletionStore,
    request: DeleteRequest,
) -> Result<
    super::catalog_commands::CatalogMutationStatusResponse,
    super::catalog_commands::CatalogMutationCommandError,
> {
    use super::catalog_commands::{
        CatalogCommandError, CatalogMutationCommandError as Failure, CatalogMutationStatusResponse,
    };
    let request = request
        .decode()
        .map_err(|_| Failure::Domain(CatalogCommandError::InvalidInput))?;
    let operation = request.request_id;
    let conversation = *request.conversation_id.as_uuid();
    let status = catalog.deletion_status(request).await?;
    Ok(CatalogMutationStatusResponse::new(
        operation,
        conversation,
        status,
        "agent.conversation.delete",
    ))
}
#[tauri::command]
pub async fn agent_read_conversation_deletion_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: DeleteRequest,
) -> Result<
    super::catalog_commands::CatalogMutationStatusResponse,
    super::catalog_commands::CatalogMutationCommandError,
> {
    execute_status(catalog.inner(), request).await
}

#[cfg(test)]
#[path = "deletion_commands/tests.rs"]
mod tests;
