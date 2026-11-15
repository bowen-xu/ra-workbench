use super::*;
use research_agent_core::conversation::{
    ConversationSearch, ConversationSearchHit, ConversationSearchPage, ConversationSearchQuery,
};

impl ConversationSearch for SqliteConversationCatalog {
    fn search(&self, query: ConversationSearchQuery) -> CatalogFuture<'_, ConversationSearchPage> {
        Box::pin(async move {
            let term = query.term.trim();
            if term.is_empty()
                || term.chars().count() > 200
                || term.contains('\0')
                || query.scope.limit == 0
                || query.scope.limit > 100
            {
                return Err(CatalogError::InvalidInput);
            }
            if let Some(binding) = &query.scope.binding {
                validate_binding(binding)?;
            }
            let term = term.to_owned();
            self.run(move |connection| {
                let scope = query.scope;
                let mut statement = connection
                    .prepare(include_str!("search.sql"))
                    .map_err(db_error)?;
                let rows = statement
                    .query_map(
                        params![
                            scope.archived,
                            scope.before.as_ref().map(|c| &c.created_at),
                            scope.before.as_ref().map(|c| c.id.as_uuid().to_string()),
                            i64::from(scope.limit) + 1,
                            scope.binding.as_ref().map(|b| binding_kind(b.kind)),
                            scope.binding.as_ref().map(|b| &b.resource_id),
                            term,
                            scope.internal,
                        ],
                        |row| {
                            Ok((
                                read_row(row)?,
                                row.get::<_, String>(7)?,
                                row.get::<_, Option<String>>(8)?,
                                row.get::<_, Option<String>>(9)?,
                                row.get::<_, Option<i64>>(10)?,
                                row.get::<_, bool>(11)?,
                            ))
                        },
                    )
                    .map_err(db_error)?;
                let mut hits = Vec::new();
                for row in rows {
                    let (summary, excerpt, item, turn, ordinal, completed) =
                        row.map_err(db_error)?;
                    let conversation = decode(summary)?;
                    let start_seq = ordinal
                        .map(|n| {
                            u64::try_from(n)
                                .ok()
                                .and_then(|n| n.checked_add(1))
                                .filter(|n| *n <= conversation.durable_count)
                                .ok_or(CatalogError::Corrupt)
                        })
                        .transpose()?;
                    hits.push(ConversationSearchHit {
                        conversation,
                        excerpt,
                        item_id: item.map(parse_identity).transpose()?,
                        turn_id: turn.map(parse_identity).transpose()?,
                        start_seq,
                        completed,
                    });
                }
                let more = hits.len() > usize::from(scope.limit);
                hits.truncate(usize::from(scope.limit));
                let next = if more {
                    hits.last().map(|h| CatalogCursor {
                        id: h.conversation.id,
                        created_at: h.conversation.created_at.clone(),
                    })
                } else {
                    None
                };
                Ok(ConversationSearchPage { hits, next })
            })
            .await
        })
    }
}
fn parse_identity(value: String) -> Result<Uuid, CatalogError> {
    Uuid::parse_str(&value)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(CatalogError::Corrupt)
}
