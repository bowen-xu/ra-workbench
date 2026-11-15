//! Status only; identity and current task authorization come from parent audit.
use super::*;
use std::collections::HashSet;
use uuid::Uuid;

pub(crate) fn record(
    db: &Connection,
    audit_seq: i64,
    handlers: &[(Uuid, Option<CompactionHookOutcome>)],
) -> rusqlite::Result<()> {
    let mut ids = HashSet::new();
    if handlers.len() > 32
        || handlers
            .iter()
            .any(|(id, _)| id.is_nil() || !ids.insert(*id))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let notification: String = db.query_row(
        "select notification from agent_compaction_hook_audit where seq=?1",
        [audit_seq],
        |row| row.get(0),
    )?;
    if notification == "completed" {
        let expected: usize = db.query_row(
            "select count(*) from agent_compaction_hook_audit a join agent_compaction_hook_audit s on s.conversation_id=a.conversation_id and s.operation_id=a.operation_id and s.turn_id=a.turn_id and s.stage=a.stage and s.notification='started' join agent_compaction_hook_handlers h on h.audit_seq=s.seq where a.seq=?1",
            [audit_seq],
            |row| row.get(0),
        )?;
        if expected != handlers.len() {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    for (index, (id, outcome)) in handlers.iter().enumerate() {
        let outcome = outcome.map(|outcome| match outcome {
            CompactionHookOutcome::Continue => "continue",
            CompactionHookOutcome::Stop => "stop",
            CompactionHookOutcome::Failed => "failed",
            CompactionHookOutcome::Canceled => "canceled",
        });
        db.execute(
            "insert into agent_compaction_hook_handlers(audit_seq,handler_id,display_order,outcome) values(?1,?2,?3,?4)",
            params![audit_seq, id.to_string(), index as i64, outcome],
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "handlers/tests.rs"]
mod tests;
