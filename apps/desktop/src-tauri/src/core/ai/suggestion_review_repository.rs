use super::{error::AiError, model::AiSuggestionItemDecision};
use rusqlite::Connection;
pub(crate) fn list_suggestion_item_decisions(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiSuggestionItemDecision>, AiError> {
    let mut statement = connection.prepare(
        "select id, candidate_id, candidate_revision, suggestion_id, action,
                resolution_kind, rationale, idempotency_key, target_object_kind,
                target_object_id, target_object_revision, created_at
         from ai_suggestion_item_decisions where candidate_id = ?1 order by created_at, id",
    )?;
    Ok(statement
        .query_map([candidate_id], |row| {
            Ok(AiSuggestionItemDecision {
                id: row.get(0)?,
                candidate_id: row.get(1)?,
                candidate_revision: row.get(2)?,
                suggestion_id: row.get(3)?,
                action: row.get(4)?,
                resolution_kind: row.get(5)?,
                rationale: row.get(6)?,
                idempotency_key: row.get(7)?,
                target_object_kind: row.get(8)?,
                target_object_id: row.get(9)?,
                target_object_revision: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}
