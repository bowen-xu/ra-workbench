//! Durable lifecycle evidence, never hook output or replay instructions.
use research_agent_core::context::{
    compaction_hooks::{
        CompactionHookNotification, CompactionHookOutcome, CompactionHookRequest,
        CompactionHookStage,
    },
    compaction_strategy::CompactionPhase,
};
use rusqlite::{Connection, params};

#[path = "compaction_hook_audit/context.rs"]
pub(crate) mod context;
#[path = "compaction_hook_audit/handlers.rs"]
pub(crate) mod handlers;

pub(crate) fn record(
    db: &Connection,
    request: &CompactionHookRequest,
    notification: CompactionHookNotification,
    recorded_at_ms: i64,
) -> rusqlite::Result<()> {
    if request.operation_id.is_nil()
        || request.conversation_id.is_nil()
        || request.turn_id.is_nil()
        || request.model.provider_id.trim().is_empty()
        || request.model.model_id.trim().is_empty()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let model = serde_json::to_string(&request.model).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if model.len() > 16384 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    // The caller owns a write transaction. Even output-free audit events must
    // belong to the current task; a matching old start is not authorization.
    let history = super::context_windows::read_projection(db, request.conversation_id)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let (count, archived): (u64, bool) = db.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [request.conversation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if archived || history.active_turn() != Some(request.turn_id) || history.seq() != count {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let stage = match request.stage {
        CompactionHookStage::PreCompact => "pre_compact",
        CompactionHookStage::PostCompact => "post_compact",
        CompactionHookStage::SessionStartCompact => "session_start_compact",
    };
    let phase = match request.phase {
        CompactionPhase::Manual => "manual",
        CompactionPhase::PreTurn => "pre_turn",
        CompactionPhase::MidTurn => "mid_turn",
    };
    let (notification, outcome) = match notification {
        CompactionHookNotification::Started => ("started", None),
        CompactionHookNotification::Completed(outcome) => (
            "completed",
            Some(match outcome {
                CompactionHookOutcome::Continue => "continue",
                CompactionHookOutcome::Stop => "stop",
                CompactionHookOutcome::Failed => "failed",
                CompactionHookOutcome::Canceled => "canceled",
            }),
        ),
    };
    db.execute(
        "insert into agent_compaction_hook_audit(conversation_id,operation_id,turn_id,stage,phase,model_json,notification,outcome,recorded_at_ms) values(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![request.conversation_id.to_string(), request.operation_id.to_string(), request.turn_id.to_string(), stage, phase, model, notification, outcome, recorded_at_ms],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "compaction_hook_audit/tests.rs"]
mod tests;
