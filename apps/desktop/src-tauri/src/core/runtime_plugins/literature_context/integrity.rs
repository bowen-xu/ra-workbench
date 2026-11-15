//! Core-owned reference protection, shared with the original static callback.
use crate::ai::reading_support::{PaperReadingFacts, ReadingSupportError};
use rusqlite::Connection;
pub(crate) enum ProtectionError {
    Database(rusqlite::Error),
    Managed(ReadingSupportError),
}
pub(crate) fn snapshot_is_protected(
    db: &Connection,
    snapshot: &str,
) -> Result<bool, ProtectionError> {
    let referenced:bool=db.query_row("select exists(select 1 from ai_runs where context_snapshot_id=?1)
             or exists(select 1 from agent_paper_submission_scopes where paper_context_snapshot_id=?1)
             or exists(select 1 from paper_analysis_workflows where context_snapshot_id=?1)
             or exists(select 1 from agent_paper_analysis_bindings where context_snapshot_id=?1)",[snapshot],|row|row.get(0)).map_err(ProtectionError::Database)?;
    Ok(referenced
        || PaperReadingFacts::new(db)
            .managed_snapshot_references(snapshot)
            .map_err(ProtectionError::Managed)?)
}
