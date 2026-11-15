//! Read-only visible-history search; no runtime, model, or filesystem access.
use super::{
    catalog_commands::{CatalogCommandError, CursorDto, ListRequest, SummaryDto},
    repository::SqliteConversationCatalog,
};
use research_agent_core::conversation::{
    ConversationSearch, ConversationSearchPage, ConversationSearchQuery,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
#[cfg(test)]
#[path = "search_commands/tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub term: String,
    pub scope: ListRequest,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub conversation: SummaryDto,
    pub excerpt: String,
    pub item_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub start_seq: Option<u64>,
    pub completed: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub schema_version: u8,
    pub hits: Vec<SearchHitDto>,
    pub next: Option<CursorDto>,
}
impl TryFrom<ConversationSearchPage> for SearchResponse {
    type Error = CatalogCommandError;
    fn try_from(page: ConversationSearchPage) -> Result<Self, Self::Error> {
        let hits = page
            .hits
            .into_iter()
            .map(|hit| {
                let valid_anchor = match (hit.item_id, hit.turn_id, hit.start_seq) {
                    (None, None, None) => hit.completed && hit.excerpt == hit.conversation.title,
                    (Some(item), Some(turn), Some(seq)) => {
                        !item.is_nil()
                            && !turn.is_nil()
                            && seq > 0
                            && seq <= hit.conversation.durable_count
                    }
                    _ => false,
                };
                if !valid_anchor || hit.excerpt.chars().count() > 320 {
                    return Err(CatalogCommandError::Corrupt);
                }
                Ok(SearchHitDto {
                    conversation: SummaryDto::try_from(hit.conversation)?,
                    excerpt: hit.excerpt,
                    item_id: hit.item_id,
                    turn_id: hit.turn_id,
                    start_seq: hit.start_seq,
                    completed: hit.completed,
                })
            })
            .collect::<Result<_, CatalogCommandError>>()?;
        Ok(Self {
            schema_version: 1,
            hits,
            next: page.next.map(|c| CursorDto {
                id: *c.id.as_uuid(),
                created_at: c.created_at,
            }),
        })
    }
}
#[tauri::command]
pub async fn agent_search_conversations(
    catalog: State<'_, SqliteConversationCatalog>,
    request: SearchRequest,
) -> Result<SearchResponse, CatalogCommandError> {
    execute(catalog.inner(), request).await
}

/// Shared native search use case; transport adapters retain the same DTO and scope validation.
pub(crate) async fn execute(
    catalog: &dyn ConversationSearch,
    request: SearchRequest,
) -> Result<SearchResponse, CatalogCommandError> {
    let page = catalog
        .search(ConversationSearchQuery {
            term: request.term,
            scope: request.scope.query()?,
        })
        .await?;
    SearchResponse::try_from(page)
}
