//! Narrow metadata IPC. No model, filesystem or provider execution is exposed.
use super::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, CatalogCursor, CatalogError, CatalogQuery, ConversationBinding,
    ConversationCatalog, ConversationId, ConversationSummary,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
#[path = "catalog_commands/archive.rs"]
mod archive;
#[path = "catalog_commands/creation.rs"]
mod creation;
#[path = "catalog_commands/mutation.rs"]
mod mutation;
pub(crate) use mutation::CatalogMutationProofDto;
pub use mutation::{CatalogMutationCommandError, CatalogMutationStatusResponse};
#[path = "catalog_commands/rename.rs"]
mod rename;
pub use archive::{ArchiveCommandError, ArchiveProofDto, ArchiveRequest, ArchiveStatusResponse};
pub(crate) use archive::{execute_archive, execute_archive_status};
pub use creation::{
    CreateBoundCommandError, CreateBoundProofDto, CreateBoundRequest, CreateBoundStatusResponse,
};
pub(crate) use creation::{execute_create_bound, execute_create_bound_status};
pub use rename::{RenameCommandError, RenameProofDto, RenameRequest, RenameStatusResponse};
pub(crate) use rename::{execute_rename, execute_rename_status};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CursorDto {
    pub created_at: String,
    pub id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRequest {
    #[serde(default)]
    pub internal: bool,
    pub binding: Option<BindingDto>,
    pub archived: bool,
    pub limit: u16,
    pub before: Option<CursorDto>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BindingKindDto {
    Manuscript,
    Paper,
    Note,
    ReadingNote,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingDto {
    pub kind: BindingKindDto,
    pub resource_id: String,
}
impl BindingDto {
    pub(crate) fn decode(self) -> Result<ConversationBinding, CatalogCommandError> {
        if self.resource_id.trim().is_empty()
            || self.resource_id.chars().count() > 256
            || self.resource_id.contains('\0')
        {
            return Err(CatalogCommandError::InvalidInput);
        }
        Ok(ConversationBinding {
            kind: match self.kind {
                BindingKindDto::Manuscript => BindingKind::Manuscript,
                BindingKindDto::Paper => BindingKind::Paper,
                BindingKindDto::Note => BindingKind::Note,
                BindingKindDto::ReadingNote => BindingKind::ReadingNote,
            },
            resource_id: self.resource_id,
        })
    }
}
#[tauri::command]
pub async fn agent_create_bound_conversation(
    catalog: State<'_, SqliteConversationCatalog>,
    request: CreateBoundRequest,
) -> Result<CreateBoundProofDto, CreateBoundCommandError> {
    execute_create_bound(&catalog, request).await
}
#[tauri::command]
pub async fn agent_read_bound_conversation_creation_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: CreateBoundRequest,
) -> Result<CreateBoundStatusResponse, CreateBoundCommandError> {
    execute_create_bound_status(&catalog, request).await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub id: Uuid,
    pub binding: Option<BindingDto>,
}
#[tauri::command]
pub async fn agent_read_conversation_metadata(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ReadRequest,
) -> Result<MetadataResponse, CatalogCommandError> {
    let conversation = catalog
        .read(
            ConversationId::from_uuid(request.id),
            request.binding.map(BindingDto::decode).transpose()?,
        )
        .await?;
    Ok(MetadataResponse {
        schema_version: 1,
        conversation: SummaryDto::try_from(conversation)?,
    })
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDto {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub internal: bool,
    pub id: Uuid,
    pub title: String,
    pub archived: bool,
    pub durable_count: u64,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    pub schema_version: u8,
    pub conversations: Vec<SummaryDto>,
    pub next: Option<CursorDto>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataResponse {
    pub schema_version: u8,
    pub conversation: SummaryDto,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogCommandError {
    InvalidInput,
    NotFound,
    Conflict,
    Busy,
    TooLarge,
    Unavailable,
    Corrupt,
}
impl From<CatalogError> for CatalogCommandError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::InvalidInput => Self::InvalidInput,
            CatalogError::NotFound => Self::NotFound,
            CatalogError::Conflict => Self::Conflict,
            CatalogError::Busy => Self::Busy,
            CatalogError::TooLarge => Self::TooLarge,
            CatalogError::Unavailable => Self::Unavailable,
            CatalogError::Corrupt => Self::Corrupt,
        }
    }
}
fn timestamp_valid(value: &str) -> bool {
    value.len() == 24
        && value.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            10 => byte == b'T',
            13 | 16 => byte == b':',
            19 => byte == b'.',
            23 => byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}
impl TryFrom<ConversationSummary> for SummaryDto {
    type Error = CatalogCommandError;
    fn try_from(value: ConversationSummary) -> Result<Self, Self::Error> {
        if value.durable_count > 9_007_199_254_740_991
            || value.title.trim().is_empty()
            || value.title.chars().count() > 180
            || value.title.contains('\0')
            || !timestamp_valid(&value.created_at)
            || !timestamp_valid(&value.updated_at)
        {
            return Err(CatalogCommandError::Corrupt);
        }
        Ok(Self {
            id: *value.id.as_uuid(),
            title: value.title,
            archived: value.archived,
            internal: value.internal,
            durable_count: value.durable_count,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}
impl ListRequest {
    pub(crate) fn query(self) -> Result<CatalogQuery, CatalogCommandError> {
        if self.limit == 0
            || self.limit > 500
            || self
                .before
                .as_ref()
                .is_some_and(|c| !timestamp_valid(&c.created_at))
        {
            return Err(CatalogCommandError::InvalidInput);
        }
        Ok(CatalogQuery {
            internal: self.internal,
            binding: self.binding.map(BindingDto::decode).transpose()?,
            archived: self.archived,
            limit: self.limit,
            before: self.before.map(|c| CatalogCursor {
                created_at: c.created_at,
                id: ConversationId::from_uuid(c.id),
            }),
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityRequest {
    pub binding: Option<BindingDto>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityResponse {
    pub schema_version: u8,
    pub running: Vec<super::repository::ConversationActivity>,
}
#[tauri::command]
pub async fn agent_read_conversation_activity(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ActivityRequest,
) -> Result<ActivityResponse, CatalogCommandError> {
    Ok(ActivityResponse {
        schema_version: 1,
        running: catalog
            .activity(request.binding.map(BindingDto::decode).transpose()?)
            .await?,
    })
}
#[tauri::command]
pub async fn agent_list_conversations(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ListRequest,
) -> Result<ListResponse, CatalogCommandError> {
    let page = catalog.list(request.query()?).await?;
    Ok(ListResponse {
        schema_version: 1,
        conversations: page
            .conversations
            .into_iter()
            .map(SummaryDto::try_from)
            .collect::<Result<_, _>>()?,
        next: page.next.map(|c| CursorDto {
            created_at: c.created_at,
            id: *c.id.as_uuid(),
        }),
    })
}
#[tauri::command]
pub async fn agent_rename_conversation(
    catalog: State<'_, SqliteConversationCatalog>,
    request: RenameRequest,
) -> Result<RenameProofDto, RenameCommandError> {
    execute_rename(catalog.inner(), request).await
}

#[tauri::command]
pub async fn agent_read_conversation_rename_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: RenameRequest,
) -> Result<RenameStatusResponse, RenameCommandError> {
    execute_rename_status(catalog.inner(), request).await
}

#[tauri::command]
pub async fn agent_set_conversation_archived(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ArchiveRequest,
) -> Result<ArchiveProofDto, ArchiveCommandError> {
    execute_archive(catalog.inner(), request).await
}

#[tauri::command]
pub async fn agent_read_conversation_archive_status(
    catalog: State<'_, SqliteConversationCatalog>,
    request: ArchiveRequest,
) -> Result<ArchiveStatusResponse, ArchiveCommandError> {
    execute_archive_status(catalog.inner(), request).await
}

#[cfg(test)]
#[path = "catalog_commands/tests.rs"]
mod tests;
