//! Typed queue claim/settlement over the existing journal admission and SQL shell.
use super::*;
use crate::agent::repository::input_queue::queued_compaction::{
    self as queue, PrepareIntent, PrepareSource, QueuedCompactionRead, QueuedCompactionState,
    ReadIntent, SettleIntent, SettleState,
};
use crate::{
    agent::writing_context::WritingContextReader, ai::structured_resource_validation::Flow,
};
use sha2::{Digest, Sha256};

const PREPARE: &str = "agent.queue.compaction.prepare";
const SETTLE: &str = "agent.queue.compaction.settle";
pub(super) fn prepare_receipt_identity(operation: Uuid, command: &str) -> ReceiptIdentity {
    identity(operation, PREPARE, command)
}
fn identity(operation: Uuid, kind: &'static str, command: &str) -> ReceiptIdentity {
    ReceiptIdentity {
        operation_id: operation,
        kind,
        version: 1,
        fingerprint: Sha256::digest(format!("{kind}:1:{command}").as_bytes()).into(),
    }
}
fn error(
    operation: Uuid,
    fence: ReadFence,
    phase: &'static str,
    error: SqlMutationError,
) -> SqlImmediateFailure {
    match error {
        SqlMutationError::Domain(research_agent_core::conversation::CatalogError::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                phase,
                "queued_compaction_state_corrupt",
            )))
        }
        SqlMutationError::Domain(error) => SqlImmediateFailure::Domain(error),
        SqlMutationError::Sql(error) => receipts::storage(operation, fence, phase, &error),
    }
}
fn initial(error: SqlMutationError) -> SqlImmediateFailure {
    match error {
        SqlMutationError::Domain(error) => SqlImmediateFailure::Domain(error),
        SqlMutationError::Sql(_) => unreachable!("command encoding performs no SQL"),
    }
}

impl Database {
    /// This is write admission, not inspection used as permission to write.
    pub(crate) fn replay_queued_compaction_prepare(
        &self,
        command: &PrepareIntent,
    ) -> Result<Option<QueuedCompactionState>, SqlImmediateFailure> {
        let frozen = queue::prepare_command(command).map_err(initial)?;
        let identity = prepare_receipt_identity(command.request_id, &frozen);
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.request_id))?;
        let mut admission = owner
            .materialize_guard()
            .map_err(|cause| admission_error(command.request_id, cause, None))?;
        let fence = admission.fence();
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-prepare-replay",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-prepare-replay",
                "sql_transaction_already_open",
            ))));
        }
        let result = (|| {
            receipts::check_state(&db, command.request_id, fence, "queue-prepare-replay")?;
            receipts::check_cross_kind(&db, command.request_id, fence, "queue-prepare-replay")?;
            let receipt = receipts::lookup(&db, &identity, fence, "queue-prepare-replay")?;
            let saved = queue::replay_in(&db, command)
                .map_err(|cause| error(command.request_id, fence, "queue-prepare-replay", cause))?;
            match (receipt, saved) {
                (Some(_), Some(saved)) => Ok(Some(saved)),
                (None, None) => Ok(None),
                _ => Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    command.request_id,
                    fence,
                    "queue-prepare-replay",
                    "queue_and_sql_receipts_disagree",
                )))),
            }
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            admission.block();
        }
        result
    }
    pub(crate) fn prepare_queued_compaction(
        &self,
        command: &PrepareIntent,
        source: PrepareSource,
    ) -> Result<QueuedCompactionState, SqlImmediateFailure> {
        self.prepare_queued_compaction_owned(
            command,
            source,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_queued_compaction_owned(
        &self,
        command: &PrepareIntent,
        source: PrepareSource,
        #[cfg(test)] fault: Fault,
    ) -> Result<QueuedCompactionState, SqlImmediateFailure> {
        self.prepare_queued_compaction_attempt(
            command,
            source,
            None,
            #[cfg(test)]
            fault,
        )
        .map_err(|flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => unreachable!("static queue preparation cannot pause for Native"),
        })
    }
    pub(crate) fn prepare_queued_compaction_context(
        &self,
        command: &PrepareIntent,
        source: PrepareSource,
        writing: &mut dyn WritingContextReader,
    ) -> writing_source::SourceResult<QueuedCompactionState> {
        self.prepare_queued_compaction_attempt(
            command,
            source,
            Some(writing),
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_queued_compaction_attempt(
        &self,
        command: &PrepareIntent,
        source: PrepareSource,
        writing: Option<&mut dyn WritingContextReader>,
        #[cfg(test)] fault: Fault,
    ) -> writing_source::SourceResult<QueuedCompactionState> {
        let frozen = queue::prepare_command(command).map_err(initial)?;
        let identity = prepare_receipt_identity(command.request_id, &frozen);
        let mut result = None;
        let mut pause =
            writing_source::SourcePause::new(command.request_id, "queued-compaction-source-pause");
        let executed = self.execute_sql(
            command.request_id,
            |tx, fence| {
                receipts::check_state(tx, command.request_id, fence, "queue-prepare")?;
                receipts::check_cross_kind(tx, command.request_id, fence, "queue-prepare")?;
                if let Some(receipt) = receipts::lookup(tx, &identity, fence, "queue-prepare")? {
                    result = Some(
                        queue::replay_in(tx, command)
                            .map_err(|cause| {
                                error(command.request_id, fence, "queue-prepare-replay", cause)
                            })?
                            .ok_or_else(|| {
                                SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                                    command.request_id,
                                    fence,
                                    "queue-prepare",
                                    "queue_receipt_missing",
                                )))
                            })?,
                    );
                    return Ok(receipt);
                }
                if queue::replay_in(tx, command)
                    .map_err(|cause| error(command.request_id, fence, "queue-prepare", cause))?
                    .is_some()
                {
                    return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                        command.request_id,
                        fence,
                        "queue-prepare",
                        "sql_receipt_missing",
                    ))));
                }
                let previous_retirement = if let Some(previous) = command.previous_request_id {
                    let read = ReadIntent {
                        conversation_id: command.conversation_id,
                        manuscript_id: command.manuscript_id,
                        input_id: command.input_id,
                        request_id: Some(previous),
                    };
                    match queue::read_in(tx, &read).map_err(|cause| {
                        error(command.request_id, fence, "queue-prepare-previous", cause)
                    })? {
                        Some(QueuedCompactionRead::Compact(state)) => {
                            super::preview_cancel::read_retirement_in(tx, &read, &state, fence)?
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let prepared = match writing {
                    Some(writing) => queue::prepare_with_context_in(
                        tx,
                        command,
                        source,
                        previous_retirement.as_deref(),
                        Some(writing),
                    ),
                    None => queue::prepare_in(tx, command, source, previous_retirement.as_deref())
                        .map_err(Flow::original),
                }
                .map_err(|flow| {
                    flow.map_error(|cause| error(command.request_id, fence, "queue-prepare", cause))
                });
                result = Some(pause.capture(prepared, fence)?);
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        );
        pause.finish(
            executed.map(|_| result.expect("successful prepare transaction sets its result")),
        )
    }
    pub(crate) fn settle_queued_compaction(
        &self,
        command: &SettleIntent,
    ) -> Result<SettleState, SqlImmediateFailure> {
        self.settle_queued_compaction_owned(
            command,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn settle_queued_compaction_owned(
        &self,
        command: &SettleIntent,
        #[cfg(test)] fault: Fault,
    ) -> Result<SettleState, SqlImmediateFailure> {
        let frozen = queue::settle_command(command).map_err(initial)?;
        let identity = identity(command.operation_id, SETTLE, &frozen);
        let mut result = None;
        self.execute_sql(command.operation_id,|tx,fence|{
            receipts::check_state(tx,command.operation_id,fence,"queue-settle")?;
            receipts::check_cross_kind(tx,command.operation_id,fence,"queue-settle")?;
            let old=receipts::lookup(tx,&identity,fence,"queue-settle")?;
            let business:bool=tx.query_row("select exists(select 1 from agent_input_queue_receipts where request_id=?1)",[command.operation_id.to_string()],|row|row.get(0)).map_err(|cause|receipts::storage(command.operation_id,fence,"queue-settle",&cause))?;
            if old.is_some()!=business{return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(command.operation_id,fence,"queue-settle","queue_and_sql_receipts_disagree"))));}
            result=Some(queue::settle_in(tx,command).map_err(|cause|error(command.operation_id,fence,"queue-settle",cause))?);
            if let Some(old)=old{return Ok(old);}
            receipts::insert(tx,&identity,fence)
        },#[cfg(test)] fault)?;
        Ok(result.expect("successful typed transaction sets its result"))
    }
    /// Read never materializes, mutates, resumes or treats missing evidence as
    /// noncommit while a write owner is blocked or a transaction remains open.
    pub(crate) fn read_queued_compaction(
        &self,
        command: &ReadIntent,
    ) -> Result<Option<QueuedCompactionRead>, SqlImmediateFailure> {
        let operation = command.request_id.unwrap_or(command.input_id);
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(operation))?;
        let guard = owner
            .inspection_guard()
            .map_err(|cause| admission_error(operation, cause, None))?;
        let snapshot = guard.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation,
                fence,
                "queue-read",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() || snapshot.blocked {
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation,
                fence,
                "queue-read",
                "queue_outcome_unconfirmed",
            ))));
        }
        let fence = receipts::status_fence(&db, operation, fence, snapshot.accepted_position)?;
        let mut result = queue::read_in(&db, command)
            .map_err(|cause| error(operation, fence, "queue-read", cause))?;
        if let Some(QueuedCompactionRead::Compact(state)) = &mut result {
            state.retirement =
                super::preview_cancel::read_retirement_in(&db, command, state, fence)?;
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "queued_compaction/tests.rs"]
mod tests;
