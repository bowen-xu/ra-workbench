//! Persist a queue-to-steering intent before dispatching the original input.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, Transaction, diagnostic, queue_edit, receipts,
};
use crate::agent::repository::{SqlRepositoryError, input_queue, steering};
use research_agent_core::{
    input_queue::QueueEdit,
    steering::{SteeringInput, SteeringScope},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(test)]
#[path = "steering_handoff_history_tests.rs"]
mod history_tests;

pub(crate) const FAMILY: &str = "agent.steering.handoff.prepare";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SteeringHandoffIntent {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub steering_input_id: Uuid,
    pub queue_input_id: Uuid,
    pub expected_revision: u64,
    pub expected_epoch: Option<Uuid>,
    pub scope: queue_edit::QueueEditScope,
    pub queue_index: u8,
    pub queue_paused: bool,
    pub input_paused: bool,
    pub origin: Option<research_agent_core::conversation::ConversationBinding>,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}

pub(crate) struct SteeringHandoffPrepared {
    pub intent: SteeringHandoffIntent,
    pub receipt: SqlImmediateReceipt,
    pub history: SteeringHandoffHistory,
}

/// An incomplete or conflicting event sequence is never proof of consumption.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SteeringHandoffHistory {
    Unconfirmed,
    Confirmed,
    IncompleteOrConflicting,
}

impl SteeringHandoffIntent {
    pub(super) fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        let valid_scope = match &self.scope {
            queue_edit::QueueEditScope::Context => true,
            queue_edit::QueueEditScope::Manuscript { manuscript_id } => !manuscript_id.is_nil(),
            queue_edit::QueueEditScope::Note { resource_id }
            | queue_edit::QueueEditScope::ReadingNote { resource_id } => !resource_id.is_nil(),
            queue_edit::QueueEditScope::Paper { paper_id } => {
                !paper_id.trim().is_empty() && paper_id.len() <= 256 && !paper_id.contains('\0')
            }
        };
        let input = SteeringInput {
            scope: SteeringScope {
                conversation_id: self.conversation_id,
                turn_id: self.turn_id,
            },
            input_id: self.steering_input_id,
            text: self.text.clone(),
            user_input: self.user_input.clone(),
            skill_instructions: vec![],
        };
        if self.operation_id.is_nil()
            || self.queue_input_id.is_nil()
            || self.expected_epoch.is_some_and(|epoch| epoch.is_nil())
            || self.expected_revision >= 9_007_199_254_740_991
            || !valid_scope
            || input.validate().is_err()
        {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    self.operation_id,
                    None,
                    "steering-handoff-validate",
                    "invalid_handoff_intent",
                ),
            )));
        }
        let bytes = serde_json::to_vec(&(FAMILY, 1u32, self)).map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                self.operation_id,
                None,
                "steering-handoff-validate",
                "handoff_encoding_failed",
            )))
        })?;
        Ok(ReceiptIdentity {
            operation_id: self.operation_id,
            kind: FAMILY,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

impl Database {
    pub(crate) fn prepare_steering_handoff(
        &self,
        intent: &SteeringHandoffIntent,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = intent.identity()?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| register(tx, fence, intent, &identity),
            #[cfg(test)]
            super::Fault::None,
        )
    }

    /// Observational only: a missing row never starts, removes, or resends input.
    pub(crate) fn inspect_steering_handoffs(
        &self,
        conversation: Uuid,
        expected_epoch: Option<Uuid>,
    ) -> Result<(Uuid, Vec<SteeringHandoffPrepared>), SqlImmediateFailure> {
        use super::{admission_error, unmanaged};
        if conversation.is_nil() || expected_epoch.is_some_and(|epoch| epoch.is_nil()) {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    conversation,
                    None,
                    "steering-handoff-status",
                    "invalid_handoff_scope",
                ),
            )));
        }
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(conversation))?;
        let mut admission = owner
            .materialize_guard()
            .map_err(|error| admission_error(conversation, error, None))?;
        let fence = admission.fence();
        if expected_epoch.is_some_and(|epoch| epoch != fence.epoch) {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                conversation,
                fence,
                "steering-handoff-status",
                "data_epoch_changed",
            ))));
        }
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                conversation,
                fence,
                "steering-handoff-status",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                conversation,
                fence,
                "steering-handoff-status",
                "handoff_owner_unconfirmed",
            ))));
        }
        if let Err(error) =
            receipts::check_state(&db, conversation, fence, "steering-handoff-status")
        {
            if matches!(error, SqlImmediateFailure::Corrupt(_)) {
                admission.block();
            }
            return Err(error);
        }
        // UUID order is unrelated to either registration or steering admission.
        // Preserve registration order for unresolved records and retain the
        // actual admission ordinal for records with a matching frozen input.
        let mut statement=db.prepare("select h.steering_input_id,h.queue_input_id,h.operation_id,h.epoch,h.intent_json,h.rowid,i.turn_id,i.text,i.ordinal from agent_steering_handoffs h left join agent_steering_inputs i on i.input_id=h.steering_input_id where h.conversation_id=?1 order by h.rowid limit 65").map_err(|error|receipts::storage(conversation,fence,"steering-handoff-status",&error))?;
        let rows = statement
            .query_map([conversation.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                ))
            })
            .map_err(|error| {
                receipts::storage(conversation, fence, "steering-handoff-status", &error)
            })?;
        let corrupt = || {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                conversation,
                fence,
                "steering-handoff-status",
                "handoff_record_invalid",
            )))
        };
        let mut items = Vec::new();
        let mut turn_order = std::collections::HashMap::new();
        for row in rows {
            let (
                steer,
                queue,
                operation,
                epoch,
                json,
                registered,
                admitted_turn,
                admitted_text,
                ordinal,
            ) = row.map_err(|error| {
                receipts::storage(conversation, fence, "steering-handoff-status", &error)
            })?;
            if items.len() >= 64 {
                return Err(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                    conversation,
                    fence,
                    "steering-handoff-status",
                    "handoff_page_limit_exceeded",
                ))));
            }
            let intent: SteeringHandoffIntent =
                serde_json::from_str(&json).map_err(|_| corrupt())?;
            let identity = intent.identity().map_err(|_| corrupt())?;
            if intent.conversation_id != conversation
                || intent.steering_input_id.to_string() != steer
                || intent.queue_input_id.to_string() != queue
                || intent.operation_id.to_string() != operation
                || epoch != fence.epoch.to_string()
                || serde_json::to_string(&intent).map_err(|_| corrupt())? != json
            {
                return Err(corrupt());
            }
            receipts::check_cross_kind(&db, intent.operation_id, fence, "steering-handoff-status")?;
            let receipt = receipts::lookup(&db, &identity, fence, "steering-handoff-status")?
                .ok_or_else(corrupt)?;
            let input = SteeringInput {
                scope: SteeringScope {
                    conversation_id: intent.conversation_id,
                    turn_id: intent.turn_id,
                },
                input_id: intent.steering_input_id,
                text: intent.text.clone(),
                user_input: intent.user_input.clone(),
                skill_instructions: vec![],
            };
            let history = history(&db, &input).map_err(|_| {
                SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                    conversation,
                    fence,
                    "steering-handoff-history",
                    "handoff_history_unavailable",
                )))
            })?;
            let turn_id = intent.turn_id.to_string();
            let matching_admission = admitted_turn.as_deref() == Some(turn_id.as_str())
                && admitted_text.as_deref() == Some(intent.text.as_str())
                && ordinal.is_some_and(|value| value >= 0);
            let admitted_ordinal = if matching_admission {
                match steering::resources::read(&db, intent.steering_input_id) {
                    Ok((resources, _)) if resources == intent.user_input => ordinal,
                    Ok(_) | Err(research_agent_core::steering::SteeringError::Invalid) => None,
                    Err(_) => {
                        return Err(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                            conversation,
                            fence,
                            "steering-handoff-status",
                            "handoff_admission_unavailable",
                        ))));
                    }
                }
            } else {
                None
            };
            let first = *turn_order.entry(intent.turn_id).or_insert(registered);
            items.push((
                first,
                admitted_ordinal,
                registered,
                SteeringHandoffPrepared {
                    intent,
                    receipt,
                    history,
                },
            ));
        }
        // A turn's admitted entries have a reliable common ordering. Remaining
        // registrations follow in registration order; their eventual admission
        // status and order are unknown and are never inferred from this list.
        items.sort_by_key(|(turn, ordinal, registered, _)| {
            (
                *turn,
                ordinal.is_none(),
                ordinal.unwrap_or(*registered),
                *registered,
            )
        });
        Ok((
            fence.epoch,
            items.into_iter().map(|(_, _, _, item)| item).collect(),
        ))
    }
}

pub(super) fn history(
    db: &rusqlite::Connection,
    input: &SteeringInput,
) -> Result<SteeringHandoffHistory, research_agent_core::steering::SteeringError> {
    match steering::consumption::is_consumed(db, input) {
        Ok(true) => Ok(SteeringHandoffHistory::Confirmed),
        Ok(false) => Ok(SteeringHandoffHistory::Unconfirmed),
        Err(research_agent_core::steering::SteeringError::Invalid) => {
            Ok(SteeringHandoffHistory::IncompleteOrConflicting)
        }
        Err(error) => Err(error),
    }
}

fn register(
    tx: &Transaction<'_>,
    fence: ReadFence,
    intent: &SteeringHandoffIntent,
    identity: &ReceiptIdentity,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = intent.operation_id;
    let conflict = |reason| {
        SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation,
            fence,
            "steering-handoff-prepare",
            reason,
        )))
    };
    if intent
        .expected_epoch
        .is_some_and(|epoch| epoch != fence.epoch)
    {
        return Err(conflict("data_epoch_changed"));
    }
    receipts::check_state(tx, operation, fence, "steering-handoff-prepare")?;
    receipts::check_cross_kind(tx, operation, fence, "steering-handoff-prepare")?;
    let queue_intent = queue_edit::QueueEditIntent {
        conversation_id: intent.conversation_id,
        request_id: operation,
        expected_revision: intent.expected_revision,
        expected_epoch: intent.expected_epoch,
        scope: intent.scope.clone(),
        edit: QueueEdit::Remove(intent.queue_input_id),
    };
    queue_edit::authorization::authorize(tx, &queue_intent).map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-authorize", &error)
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_scope_unavailable"),
    })?;
    let cancellation = super::steering_handoff_cancel::identity(
        &super::steering_handoff_finish::FinishSteeringHandoff {
            intent: intent.clone(),
            expected_epoch: fence.epoch,
        },
    )?;
    receipts::check_cross_kind(
        tx,
        cancellation.operation_id,
        fence,
        "steering-handoff-prepare",
    )?;
    if receipts::lookup(tx, &cancellation, fence, "steering-handoff-prepare")?.is_some() {
        return Err(conflict("handoff_registration_canceled"));
    }
    let saved: Option<(String, String)> = tx
        .query_row(
            "select epoch,intent_json from agent_steering_handoffs where steering_input_id=?1",
            [intent.steering_input_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| receipts::storage(operation, fence, "steering-handoff-read", &error))?;
    let encoded = serde_json::to_string(intent).map_err(|_| conflict("handoff_encoding_failed"))?;
    let receipt = receipts::lookup(tx, identity, fence, "steering-handoff-prepare")?;
    match (saved, receipt) {
        (Some((epoch, json)), Some(receipt))
            if epoch == fence.epoch.to_string() && json == encoded =>
        {
            return Ok(receipt);
        }
        (None, None) => {}
        (None, Some(_)) => {
            let request = super::steering_handoff_finish::FinishSteeringHandoff {
                intent: intent.clone(),
                expected_epoch: fence.epoch,
            };
            let finished = request.identity()?;
            if receipts::lookup(tx, &finished, fence, "steering-handoff-prepare")?.is_some() {
                return Err(conflict("handoff_already_finished"));
            }
            let released = super::steering_handoff_release::identity(&request)?;
            if receipts::lookup(tx, &released, fence, "steering-handoff-prepare")?.is_some() {
                return Err(conflict("handoff_already_released"));
            }
            return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-prepare",
                "handoff_receipt_mismatch",
            ))));
        }
        (Some((epoch, json)), _) if epoch != fence.epoch.to_string() || json != encoded => {
            return Err(conflict("handoff_identity_changed"));
        }
        _ => {
            return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-prepare",
                "handoff_receipt_mismatch",
            ))));
        }
    }
    let scope = SteeringScope {
        conversation_id: intent.conversation_id,
        turn_id: intent.turn_id,
    };
    steering::require_active(tx, scope).map_err(|error| match error {
        research_agent_core::steering::SteeringError::Unavailable => {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-turn",
                "handoff_turn_storage_unavailable",
            )))
        }
        _ => conflict("handoff_turn_unavailable"),
    })?;
    let queue = input_queue::load_sql(tx, intent.conversation_id).map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-queue", &error)
        }
        SqlRepositoryError::Domain(input_queue::Error::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-queue",
                "handoff_queue_state_corrupt",
            )))
        }
        SqlRepositoryError::Domain(input_queue::Error::Storage) => {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-queue",
                "handoff_queue_storage_unavailable",
            )))
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_queue_unavailable"),
    })?;
    let matching = queue
        .inputs()
        .get(usize::from(intent.queue_index))
        .is_some_and(|input| {
            input.id == intent.queue_input_id
                && input.text == intent.text
                && input.user_input == intent.user_input
                && input.origin == intent.origin
        });
    if queue.revision() != intent.expected_revision
        || queue.preparing().is_some()
        || !matching
        || queue.paused() != intent.queue_paused
        || queue.input_paused(intent.queue_input_id) != intent.input_paused
    {
        return Err(conflict("handoff_queue_changed"));
    }
    let already:bool=tx.query_row("select exists(select 1 from agent_steering_handoffs where conversation_id=?1 and queue_input_id=?2)",params![intent.conversation_id.to_string(),intent.queue_input_id.to_string()],|row|row.get(0)).map_err(|error|receipts::storage(operation,fence,"steering-handoff-read",&error))?;
    if already {
        return Err(conflict("handoff_queue_already_owned"));
    }
    tx.execute("insert into agent_steering_handoffs(steering_input_id,conversation_id,queue_input_id,operation_id,epoch,intent_json) values(?1,?2,?3,?4,?5,?6)",params![intent.steering_input_id.to_string(),intent.conversation_id.to_string(),intent.queue_input_id.to_string(),operation.to_string(),fence.epoch.to_string(),encoded]).map_err(|error|receipts::storage(operation,fence,"steering-handoff-insert",&error))?;
    receipts::insert(tx, identity, fence)
}
