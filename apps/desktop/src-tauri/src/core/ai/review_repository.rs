use super::{candidate_repository::get_candidate, error::AiError};
use rusqlite::{Connection, params};
use uuid::Uuid;
pub(crate) fn record_evidence_inspection(
    connection: &Connection,
    candidate_id: &str,
    candidate_revision: i64,
    evidence_item_id: &str,
) -> Result<bool, AiError> {
    let candidate = get_candidate(connection, candidate_id)?;
    if candidate.current_revision != candidate_revision
        || !candidate
            .evidence_bundle
            .items
            .iter()
            .any(|item| item.id == evidence_item_id)
    {
        return Err(AiError::InvalidInput(
            "证据查看事件与当前候选版本不匹配".to_owned(),
        ));
    }
    connection.execute(
        "insert into ai_evidence_inspections
         (id, candidate_id, candidate_revision, evidence_item_id, viewed_at)
         values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(candidate_id, candidate_revision, evidence_item_id) do nothing",
        params![
            Uuid::new_v4().to_string(),
            candidate_id,
            candidate_revision,
            evidence_item_id
        ],
    )?;
    Ok(true)
}
