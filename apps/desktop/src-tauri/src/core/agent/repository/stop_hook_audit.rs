//! Transactional Stop hook admission and result facts. The conversation task
//! owns serialization and must commit through the data-interface write guard.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::{
    protocol::ModelIdentity,
    runner::{StopEvent, StopHandlerOutput, StopHandlerStatus},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Admission {
    pub invocation_id: Uuid,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub event: StopEvent,
    pub model: ModelIdentity,
    pub handlers: Vec<(Uuid, Uuid)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Completed {
        status: Status,
        system_message: Option<String>,
    },
    Canceled,
    NotStarted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    Completed,
    Failed { diagnostic: String },
    Stopped { reason: Option<String> },
    Blocked { feedback: String },
}
impl From<StopHandlerOutput> for Outcome {
    fn from(output: StopHandlerOutput) -> Self {
        let status = match output.status {
            StopHandlerStatus::Completed => Status::Completed,
            StopHandlerStatus::Failed { diagnostic } => Status::Failed { diagnostic },
            StopHandlerStatus::Stopped { reason } => Status::Stopped { reason },
            StopHandlerStatus::Blocked { feedback } => Status::Blocked { feedback },
        };
        Self::Completed {
            status,
            system_message: output.system_message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub canceled: bool,
    pub outcomes: Vec<(Uuid, Uuid, Outcome)>,
}

pub struct Recorded {
    pub admission: Admission,
    pub boundary_seq: u64,
    pub finished: Option<Finished>,
}

pub struct SqliteStopHookAudit {
    db: SharedDatabase,
}
impl SqliteStopHookAudit {
    #[cfg(target_os = "macos")]
    pub(crate) fn belongs_to(&self, service: &crate::agent::service::AgentSessionService) -> bool {
        service.owns_data(&self.db) && self.db.logged()
    }
    #[allow(
        dead_code,
        reason = "the task owner will inject this adapter when Stop is wired"
    )]
    pub(crate) fn new(db: impl IntoDatabase) -> Self {
        Self {
            db: db.into_database(),
        }
    }

    async fn transaction<T: Send + 'static>(
        &self,
        behavior: rusqlite::TransactionBehavior,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> rusqlite::Result<T> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            crate::data_interface::guarded_transaction::run(&mut db, |connection| {
                let mut tx = match rusqlite::Transaction::new_unchecked(connection, behavior) {
                    Ok(tx) => tx,
                    Err(error) => return (Err(error), !connection.is_autocommit()),
                };
                tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
                match operation(&tx) {
                    Err(error) => {
                        let rollback = tx.rollback();
                        let unknown = rollback.is_err() || !connection.is_autocommit();
                        (Err(error), unknown)
                    }
                    Ok(value) => {
                        if let Err(error) = tx.commit() {
                            if !connection.is_autocommit() {
                                let _ = connection.execute_batch("rollback");
                            }
                            return (Err(error), true);
                        }
                        if !connection.is_autocommit() {
                            return (Err(rusqlite::Error::InvalidQuery), true);
                        }
                        (Ok(value), false)
                    }
                }
            })
        })
        .await
        .map_err(|_| rusqlite::Error::InvalidQuery)?
    }

    pub async fn admit(&self, admission: Admission, at_ms: i64) -> rusqlite::Result<()> {
        self.transaction(rusqlite::TransactionBehavior::Immediate, move |db| {
            admit(db, &admission, at_ms)
        })
        .await
    }
    pub async fn finish(
        &self,
        admission: Admission,
        result: Finished,
        at_ms: i64,
    ) -> rusqlite::Result<()> {
        self.transaction(rusqlite::TransactionBehavior::Immediate, move |db| {
            finish(db, &admission, &result, at_ms)
        })
        .await
    }
    pub async fn read(
        &self,
        conversation: Uuid,
        invocation: Uuid,
    ) -> rusqlite::Result<Option<Recorded>> {
        self.transaction(rusqlite::TransactionBehavior::Deferred, move |db| {
            read(db, conversation, invocation)
        })
        .await
    }

    /// Read a result and its active journal boundary in one snapshot. The
    /// eventual journal write must recheck the boundary in its own transaction.
    #[allow(
        dead_code,
        reason = "the task owner will consume recorded Stop results"
    )]
    pub async fn read_active(
        &self,
        conversation: Uuid,
        invocation: Uuid,
    ) -> rusqlite::Result<Option<Recorded>> {
        self.transaction(rusqlite::TransactionBehavior::Deferred, move |db| {
            read_active(db, conversation, invocation)
        })
        .await
    }
}

fn read_active(
    db: &Connection,
    conversation: Uuid,
    invocation: Uuid,
) -> rusqlite::Result<Option<Recorded>> {
    let Some(recorded) = read(db, conversation, invocation)? else {
        return Ok(None);
    };
    let current = active(db, &recorded.admission)?;
    if recorded.boundary_seq > current {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Some(recorded))
}

/// Verify a proposed prompt against committed audit facts in the caller's
/// snapshot. This does not replace the session owner's ordering check.
#[cfg(target_os = "macos")]
pub(crate) fn validate_prompt_source(
    db: &Connection,
    event: &research_agent_core::protocol::AgentEvent,
    history: &research_agent_core::protocol::ConversationProjection,
    unmaterialized_model_starts: &[(Uuid, u64)],
) -> Result<(), super::JournalError> {
    use crate::agent::compaction_hook_config::native::StopResolution;
    use research_agent_core::protocol::{EventPayload, ItemKind};
    let EventPayload::ItemAccepted {
        turn_id,
        kind: ItemKind::HookPrompt {
            invocation_id,
            fragments,
        },
        ..
    } = event.payload()
    else {
        return Ok(());
    };
    let conversation = *event.conversation_id().as_uuid();
    let recorded = read(db, conversation, *invocation_id)
        .map_err(|_| super::JournalError::Unavailable)?
        .ok_or(super::JournalError::InvalidInput)?;
    if recorded.admission.turn_id != *turn_id
        || recorded.boundary_seq >= event.seq()
        || history.active_turn() != Some(*turn_id)
        || history
            .turn(*turn_id)
            .is_none_or(|turn| turn.model != recorded.admission.model)
    {
        return Err(super::JournalError::InvalidInput);
    }
    // The admission watermark denotes the end of the model response being
    // checked. A later sample makes its feedback stale, even if the same turn
    // is still active and its audit record remains intact.
    let stale_in_sql: bool = db.query_row(
        "select exists(select 1 from agent_journal_records where conversation_id=?1 and ordinal>=?2 and ordinal<?3 and json_extract(payload_json,'$.payload.type')='modelRequestStarted' and json_extract(payload_json,'$.payload.turnId')=?4)",
        params![conversation.to_string(), recorded.boundary_seq, event.seq() - 1, turn_id.to_string()], |row| row.get(0),
    ).map_err(|_| super::JournalError::Unavailable)?;
    if stale_in_sql
        || unmaterialized_model_starts.iter().any(|(turn, seq)| {
            turn == turn_id && *seq > recorded.boundary_seq && *seq < event.seq()
        })
    {
        return Err(super::JournalError::InvalidInput);
    }
    let Some(StopResolution::FollowUp { parts }) = StopResolution::from_recorded(&recorded) else {
        return Err(super::JournalError::InvalidInput);
    };
    if parts.len() != fragments.len()
        || parts.iter().zip(fragments).any(|(part, fragment)| {
            part.run_id != fragment.run_id || part.input_text != fragment.input_text
        })
    {
        return Err(super::JournalError::InvalidInput);
    }
    let consumed: bool = db.query_row(
        "select exists(select 1 from agent_journal_records where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='itemAccepted' and json_extract(payload_json,'$.payload.kind.type')='hookPrompt' and json_extract(payload_json,'$.payload.kind.invocationId')=?2)",
        params![conversation.to_string(), invocation_id.to_string()], |row| row.get(0),
    ).map_err(|_| super::JournalError::Unavailable)?;
    if consumed {
        return Err(super::JournalError::InvalidInput);
    }
    Ok(())
}

/// Caller supplies an authorized read transaction; incomplete admissions are
/// returned as incomplete and must never be treated as permission to rerun.
fn read(
    db: &Connection,
    conversation: Uuid,
    invocation: Uuid,
) -> rusqlite::Result<Option<Recorded>> {
    if db.is_autocommit() || conversation.is_nil() || invocation.is_nil() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let row: Option<(String,String,String,String,u64)> = db.query_row("select conversation_id,turn_id,event_name,model_json,boundary_seq from agent_stop_hook_admissions where invocation_id=?1 and conversation_id=?2", params![invocation.to_string(), conversation.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((conversation, turn, event, model, boundary_seq)) = row else {
        return Ok(None);
    };
    let event = match event.as_str() {
        "stop" => StopEvent::Stop,
        "subagent_stop" => StopEvent::SubagentStop,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let parse = |value: &str| {
        Uuid::parse_str(value)
            .ok()
            .filter(|id| !id.is_nil())
            .ok_or(rusqlite::Error::InvalidQuery)
    };
    let mut admission = Admission {
        invocation_id: invocation,
        conversation_id: parse(&conversation)?,
        turn_id: parse(&turn)?,
        event,
        model: serde_json::from_str(&model).map_err(|_| rusqlite::Error::InvalidQuery)?,
        handlers: vec![],
    };
    let canceled: Option<bool> = db
        .query_row(
            "select canceled from agent_stop_hook_finishes where invocation_id=?1",
            [invocation.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let mut statement = db.prepare("select h.ordinal,h.handler_id,h.run_id,r.outcome_json from agent_stop_hook_handlers h left join agent_stop_hook_results r on r.invocation_id=h.invocation_id and r.ordinal=h.ordinal and r.run_id=h.run_id where h.invocation_id=?1 order by h.ordinal")?;
    let rows = statement.query_map([invocation.to_string()], |r| {
        Ok((
            r.get::<_, usize>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut outcomes = Vec::new();
    for row in rows {
        let (ordinal, handler, run, outcome) = row?;
        if ordinal != admission.handlers.len() || ordinal >= 32 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let (handler, run) = (parse(&handler)?, parse(&run)?);
        admission.handlers.push((handler, run));
        match (canceled, outcome) {
            (Some(canceled), Some(encoded)) => {
                let outcome: Outcome =
                    serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
                if (!canceled && matches!(outcome, Outcome::Canceled | Outcome::NotStarted))
                    || matches!(&outcome,Outcome::Completed { status: Status::Blocked { feedback }, .. } if feedback.trim().is_empty())
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                outcomes.push((handler, run, outcome));
            }
            (None, None) => {}
            _ => return Err(rusqlite::Error::InvalidQuery),
        }
    }
    if admission.handlers.is_empty() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Some(Recorded {
        admission,
        boundary_seq,
        finished: canceled.map(|canceled| Finished { canceled, outcomes }),
    }))
}

fn active(db: &Connection, admission: &Admission) -> rusqlite::Result<u64> {
    if db.is_autocommit()
        || admission.invocation_id.is_nil()
        || admission.conversation_id.is_nil()
        || admission.turn_id.is_nil()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let history = super::context_windows::read_projection(db, admission.conversation_id)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let (count, archived): (u64, bool) = db.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [admission.conversation_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if archived
        || history.active_turn() != Some(admission.turn_id)
        || history.seq() != count
        || history
            .turn(admission.turn_id)
            .is_none_or(|turn| turn.model != admission.model)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(count)
}

/// Must be called inside the task owner's immediate transaction. Admission is
/// not proof of process start and is never replayed automatically after crash.
fn admit(db: &Connection, admission: &Admission, at_ms: i64) -> rusqlite::Result<()> {
    let boundary = active(db, admission)?;
    let mut handlers = HashSet::new();
    let mut runs = HashSet::new();
    if at_ms < 0
        || admission.handlers.is_empty()
        || admission.handlers.len() > 32
        || admission.handlers.iter().any(|(handler, run)| {
            handler.is_nil() || run.is_nil() || !handlers.insert(*handler) || !runs.insert(*run)
        })
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let model =
        serde_json::to_string(&admission.model).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if model.len() > 16384 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let event = match admission.event {
        StopEvent::Stop => "stop",
        StopEvent::SubagentStop => "subagent_stop",
    };
    db.execute("insert into agent_stop_hook_admissions(invocation_id,conversation_id,turn_id,event_name,model_json,boundary_seq,admitted_at_ms) values(?1,?2,?3,?4,?5,?6,?7)", params![admission.invocation_id.to_string(),admission.conversation_id.to_string(),admission.turn_id.to_string(),event,model,i64::try_from(boundary).map_err(|_| rusqlite::Error::InvalidQuery)?,at_ms])?;
    for (ordinal, (handler, run)) in admission.handlers.iter().enumerate() {
        db.execute("insert into agent_stop_hook_handlers(invocation_id,ordinal,handler_id,run_id) values(?1,?2,?3,?4)", params![admission.invocation_id.to_string(), ordinal as i64, handler.to_string(), run.to_string()])?;
    }
    Ok(())
}

/// Writes all completed/canceled outcomes and the finish marker in one owner
/// transaction. Returned feedback must not be published before commit succeeds.
fn finish(
    db: &Connection,
    admission: &Admission,
    result: &Finished,
    at_ms: i64,
) -> rusqlite::Result<()> {
    active(db, admission)?;
    if at_ms < 0 || result.outcomes.len() != admission.handlers.len() || result.outcomes.iter().zip(&admission.handlers).any(|((handler,run,outcome),expected)| (*handler,*run) != *expected || (!result.canceled && matches!(outcome,Outcome::Canceled | Outcome::NotStarted)) || matches!(outcome, Outcome::Completed { status: Status::Blocked { feedback }, .. } if feedback.trim().is_empty())) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let stored: Option<(String,String,String,String)> = db.query_row("select conversation_id,turn_id,event_name,model_json from agent_stop_hook_admissions where invocation_id=?1", [admission.invocation_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let event = match admission.event {
        StopEvent::Stop => "stop",
        StopEvent::SubagentStop => "subagent_stop",
    };
    let model =
        serde_json::to_string(&admission.model).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if stored
        != Some((
            admission.conversation_id.to_string(),
            admission.turn_id.to_string(),
            event.into(),
            model,
        ))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut statement = db.prepare("select handler_id,run_id from agent_stop_hook_handlers where invocation_id=?1 order by ordinal")?;
    let rows = statement.query_map([admission.invocation_id.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let identities = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if identities.len() != admission.handlers.len()
        || identities.iter().zip(&admission.handlers).any(
            |((handler, run), (expected_handler, expected_run))| {
                *handler != expected_handler.to_string() || *run != expected_run.to_string()
            },
        )
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    drop(statement);
    for (ordinal, (_, run, outcome)) in result.outcomes.iter().enumerate() {
        let encoded = serde_json::to_string(outcome).map_err(|_| rusqlite::Error::InvalidQuery)?;
        db.execute("insert into agent_stop_hook_results(invocation_id,ordinal,run_id,outcome_json) values(?1,?2,?3,?4)", params![admission.invocation_id.to_string(),ordinal as i64,run.to_string(),encoded])?;
    }
    db.execute("insert into agent_stop_hook_finishes(invocation_id,canceled,finished_at_ms) values(?1,?2,?3)", params![admission.invocation_id.to_string(),result.canceled,at_ms])?;
    Ok(())
}

#[cfg(test)]
#[path = "stop_hook_audit/tests.rs"]
mod tests;
