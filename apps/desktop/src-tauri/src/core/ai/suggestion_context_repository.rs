use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::error::AiError;

pub(crate) use research_workbench_native_sdk::generation::SUGGESTION_CANDIDATE_KINDS;

pub(crate) fn insert_suggestion_context(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate_kind: &str,
) -> Result<(), AiError> {
    if !SUGGESTION_CANDIDATE_KINDS.contains(&candidate_kind) {
        return Err(AiError::InvalidInput("AI 建议候选类型无效".to_owned()));
    }
    transaction.execute(
        "insert into ai_suggestion_run_contexts (run_id, candidate_kind, created_at)
         values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![run_id, candidate_kind],
    )?;
    Ok(())
}

pub(crate) fn load_suggestion_context(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<String>, AiError> {
    connection
        .query_row(
            "select candidate_kind from ai_suggestion_run_contexts where run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}
