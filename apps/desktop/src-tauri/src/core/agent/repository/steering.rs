//! Same-turn admission and empty-queue sealing share an immediate SQLite
//! transaction. Unconsumed steering inputs are cleared at application startup;
//! committed conversation items remain durable.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::steering::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use uuid::Uuid;
#[path = "steering/consumption.rs"]
pub(crate) mod consumption;
#[path = "steering/recovery.rs"]
mod recovery;
pub(crate) use recovery::require_terminal;
#[path = "steering/resources.rs"]
pub(crate) mod resources;

pub struct SqliteSteeringStore {
    connection: SharedDatabase,
}
impl SqliteSteeringStore {
    /// A new runtime discards input with no matching history event.
    pub(crate) fn reconcile_startup(store: &crate::storage::AppStore) -> Result<(), SteeringError> {
        let mut db = store.connection().map_err(|_| SteeringError::Unavailable)?;
        Self::discard_unconsumed_on_startup(&mut db)
    }

    pub(crate) fn new(connection: impl IntoDatabase) -> Self {
        Self {
            connection: connection.into_database(),
        }
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, SteeringError> + Send + 'static,
    ) -> Result<T, SteeringError> {
        self.run_with_behavior(TransactionBehavior::Immediate, operation)
            .await
    }

    async fn run_with_behavior<T: Send + 'static>(
        &self,
        behavior: TransactionBehavior,
        operation: impl FnOnce(&Connection) -> Result<T, SteeringError> + Send + 'static,
    ) -> Result<T, SteeringError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock().map_err(|_| SteeringError::Unavailable)?;
            crate::data_interface::guarded_transaction::run(&mut connection, |db| {
                let mut tx = match rusqlite::Transaction::new_unchecked(db, behavior) {
                    Ok(tx) => tx,
                    Err(error) => return (Err(db_error(error)), !db.is_autocommit()),
                };
                tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
                match operation(&tx) {
                    Err(error) => {
                        let rollback = tx.rollback();
                        let unknown = rollback.is_err() || !db.is_autocommit();
                        (
                            Err(if unknown {
                                SteeringError::Unavailable
                            } else {
                                error
                            }),
                            unknown,
                        )
                    }
                    Ok(value) => {
                        if tx.commit().is_err() {
                            if !db.is_autocommit() {
                                let _ = db.execute_batch("rollback");
                            }
                            return (Err(SteeringError::Unavailable), true);
                        }
                        if !db.is_autocommit() {
                            return (Err(SteeringError::Unavailable), true);
                        }
                        (Ok(value), false)
                    }
                }
            })
        })
        .await
        .map_err(|_| SteeringError::Unavailable)?
    }

    /// Pending same-turn inputs belong to the current agent process. The Codex
    /// steering contract does not replay accepted-but-unconsumed input after
    /// the transport/session is recreated; only inputs already committed to
    /// conversation history survive that boundary.
    pub(crate) fn discard_unconsumed_on_startup(db: &mut Connection) -> Result<(), SteeringError> {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute(
            "delete from agent_steering_inputs where not exists (
                select 1 from agent_items
                where json_extract(agent_items.payload_json,'$.payload.itemId') = agent_steering_inputs.input_id
            )",
            [],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
}
impl SqliteSteeringStore {
    pub(crate) fn enqueue_checked(
        &self,
        input: SteeringInput,
        check: impl FnOnce(&Connection) -> Result<(), SteeringError> + Send + 'static,
    ) -> SteeringFuture<'_, ()> {
        self.enqueue_with_resources(input, check, |_, _| Ok(()))
    }
    pub(crate) fn enqueue_with_resources(
        &self,
        mut input: SteeringInput,
        check: impl FnOnce(&Connection) -> Result<(), SteeringError> + Send + 'static,
        prepare: impl FnOnce(&Connection, &mut SteeringInput) -> Result<(), SteeringError>
        + Send
        + 'static,
    ) -> SteeringFuture<'_, ()> {
        Box::pin(async move {
            input.validate()?;
            self.run(move |db| {
                check(db)?;
                let existing: Option<(String,String,String)> = db.query_row(
                    "select s.conversation_id, i.turn_id, i.text from agent_steering_inputs i join agent_submissions s on s.turn_id=i.turn_id where i.input_id=?1",
                    [input.input_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
                if let Some((conversation, turn, text)) = existing {
                    return if conversation == input.scope.conversation_id.to_string() && turn == input.scope.turn_id.to_string() && text == input.text && resources::read(db, input.input_id)?.0 == input.user_input { Ok(()) } else { Err(SteeringError::Conflict) };
                }
                require_active(db, input.scope)?;
                ensure_gate(db, input.scope)?;
                let (sealed, count): (bool,i64) = db.query_row("select sealed, (select count(*) from agent_steering_inputs where turn_id=?1) from agent_steering_gates where turn_id=?1", [input.scope.turn_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
                if sealed { return Err(SteeringError::Closed); }
                if count >= 64 { return Err(SteeringError::Capacity); }
                prepare(db, &mut input)?;
                input.validate()?;
                db.execute("insert into agent_steering_inputs(input_id,turn_id,ordinal,text) values(?1,?2,?3,?4)", params![input.input_id.to_string(),input.scope.turn_id.to_string(),count,input.text]).map_err(db_error)?;
                resources::insert(db, &input)?;
                Ok(())
            }).await
        })
    }
}
impl SteeringStore for SqliteSteeringStore {
    fn enqueue(&self, input: SteeringInput) -> SteeringFuture<'_, ()> {
        self.enqueue_checked(input, |_| Ok(()))
    }
    fn seal_for_stop(&self, scope: SteeringScope) -> SteeringFuture<'_, ()> {
        Box::pin(async move {
            self.run(move |db| {
                require_active(db, scope)?;
                ensure_gate(db, scope)?;
                db.execute(
                    "update agent_steering_gates set sealed=1 where turn_id=?1",
                    [scope.turn_id.to_string()],
                )
                .map_err(db_error)?;
                Ok(())
            })
            .await
        })
    }
    fn pending(
        &self,
        scope: SteeringScope,
        finish_if_empty: bool,
    ) -> SteeringFuture<'_, Vec<SteeringInput>> {
        Box::pin(async move {
            let behavior = if finish_if_empty {
                TransactionBehavior::Immediate
            } else {
                TransactionBehavior::Deferred
            };
            self.run_with_behavior(behavior, move |db| {
                require_active(db, scope)?;
                if finish_if_empty {
                    ensure_gate(db, scope)?;
                }
                let mut statement = db.prepare("select input_id,text from agent_steering_inputs where turn_id=?1 order by ordinal").map_err(db_error)?;
                let rows = statement.query_map([scope.turn_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))).map_err(db_error)?;
                let mut inputs = Vec::new();
                for row in rows {
                    let (id,text) = row.map_err(db_error)?;
                    let input = resources::input(db, scope, &id, text)?;
                    input.validate()?;
                    if !consumption::is_consumed(db, &input)? { inputs.push(input); }
                }
                if inputs.is_empty() && finish_if_empty {
                    db.execute("update agent_steering_gates set sealed=1 where turn_id=?1", [scope.turn_id.to_string()]).map_err(db_error)?;
                }
                Ok(inputs)
            }).await
        })
    }
}
fn ensure_gate(db: &Connection, scope: SteeringScope) -> Result<(), SteeringError> {
    db.execute(
        "insert or ignore into agent_steering_gates(turn_id) values(?1)",
        [scope.turn_id.to_string()],
    )
    .map_err(db_error)?;
    Ok(())
}
pub(crate) fn require_active(db: &Connection, scope: SteeringScope) -> Result<(), SteeringError> {
    let admitted: bool = db.query_row("select exists(select 1 from agent_submissions s join agent_conversations c on c.id=s.conversation_id where s.turn_id=?1 and s.conversation_id=?2 and c.archived=0)", params![scope.turn_id.to_string(),scope.conversation_id.to_string()], |r| r.get(0)).map_err(db_error)?;
    let current: Option<String> = db.query_row("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type') in ('turnStarted','turnFinished') order by ordinal desc limit 1", [scope.conversation_id.to_string()], |r| r.get(0)).optional().map_err(db_error)?;
    let event = current.and_then(|json| {
        serde_json::from_str::<research_agent_core::protocol::AgentEvent>(&json).ok()
    });
    if let Some(event) = &event
        && admitted
        && *event.conversation_id().as_uuid() == scope.conversation_id
        && let research_agent_core::protocol::EventPayload::TurnStarted { turn_id, .. } =
            event.payload()
        && *turn_id != scope.turn_id
    {
        return Err(SteeringError::ExpectedTurnMismatch {
            actual_turn_id: *turn_id,
        });
    }
    if !admitted || !event.as_ref().is_some_and(|event| *event.conversation_id().as_uuid() == scope.conversation_id && matches!(event.payload(), research_agent_core::protocol::EventPayload::TurnStarted { turn_id, .. } if *turn_id == scope.turn_id)) {
        return Err(SteeringError::Closed);
    }
    if let Some(event) = event
        && let research_agent_core::protocol::EventPayload::TurnStarted { task_kind, .. } =
            event.payload()
        && !task_kind.is_regular()
    {
        return Err(SteeringError::NotSteerable {
            task_kind: *task_kind,
        });
    }
    Ok(())
}
fn db_error(_: rusqlite::Error) -> SteeringError {
    SteeringError::Unavailable
}

#[cfg(test)]
#[path = "steering/tests.rs"]
mod tests;
