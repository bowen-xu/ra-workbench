//! Transactional next-turn snapshots and exact retry receipts. Call only on a
//! blocking executor; the host verifies resource binding inside each transaction.
use research_agent_core::conversation::ConversationBinding;
use research_agent_core::input_queue::{InputQueue, QueueEdit, QueueError, QueuedInput};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[path = "input_queue/startup.rs"]
mod startup;
pub use startup::pause_on_startup;
#[path = "input_queue/stop.rs"]
mod stop;
pub use stop::pause_for_turn_checked;
#[path = "input_queue/history_rewrite.rs"]
mod history_rewrite;
pub(crate) use history_rewrite::pause_in as pause_for_history_rewrite_in;
pub(crate) use history_rewrite::require_startable_in;
#[path = "input_queue/admission.rs"]
mod admission;
pub(crate) use admission::admit_and_resume_checked;
pub use admission::admit_checked;
pub(crate) use admission::admit_in_tx;
#[path = "input_queue/preview.rs"]
mod preview;
pub use preview::prepare_preview_checked;
pub(crate) use preview::prepare_preview_in;
pub(crate) use preview::prepare_resolved_preview_in;
#[path = "input_queue/unstarted.rs"]
mod unstarted;
pub use unstarted::{UnstartedQueueAdmission, list_unstarted_checked};
#[path = "input_queue/recovery.rs"]
mod recovery;
#[path = "input_queue/start_inspection.rs"]
pub(crate) mod start_inspection;
pub use recovery::read_pending_preview_checked;
#[path = "input_queue/cancel_preview.rs"]
mod cancel_preview;
pub use cancel_preview::cancel_preview_checked;
pub(crate) use cancel_preview::{cancel_preview_in, read_cancel_preview_receipt};
#[path = "input_queue/edit.rs"]
mod edit;
#[path = "input_queue/handoff_guard.rs"]
mod handoff_guard;
#[path = "input_queue/preview_prepare.rs"]
pub(crate) mod preview_prepare;
#[path = "input_queue/queued_compaction.rs"]
pub(crate) mod queued_compaction;
#[path = "input_queue/restore.rs"]
mod restore;
pub(crate) use edit::{
    QueueSqlError, apply_handoff_removal_in_tx_sql, apply_in_tx_sql, load_sql,
    read_edit_receipt_sql, read_frozen_edit_sql, read_legacy_edit_sql,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Queue(#[from] QueueError),
    #[error("queue storage unavailable")]
    Storage,
    #[error("queue conversation unavailable")]
    Closed,
    #[error("invalid persisted queue")]
    Corrupt,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    conversation: Uuid,
    revision: u64,
    paused: bool,
    inputs: Vec<(Uuid, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    origins: Vec<(Uuid, ConversationBinding)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    user_inputs: Vec<(Uuid, Vec<research_agent_core::user_input::UserInput>)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paused_inputs: Vec<Uuid>,
    preparing: Option<Uuid>,
}

impl Snapshot {
    fn encode(queue: &InputQueue) -> Result<String, Error> {
        serde_json::to_string(&Self {
            version: if queue
                .inputs()
                .iter()
                .any(|input| queue.input_paused(input.id))
            {
                4
            } else if queue
                .inputs()
                .iter()
                .any(|input| !input.user_input.is_empty())
            {
                3
            } else {
                2
            },
            user_inputs: queue
                .inputs()
                .iter()
                .filter(|input| !input.user_input.is_empty())
                .map(|input| (input.id, input.user_input.clone()))
                .collect(),
            conversation: queue.conversation_id(),
            revision: queue.revision(),
            paused: queue.paused(),
            paused_inputs: queue
                .inputs()
                .iter()
                .filter(|input| queue.input_paused(input.id))
                .map(|input| input.id)
                .collect(),
            preparing: queue.preparing(),
            origins: queue
                .inputs()
                .iter()
                .filter_map(|input| input.origin.clone().map(|origin| (input.id, origin)))
                .collect(),
            inputs: queue
                .inputs()
                .iter()
                .map(|input| (input.id, input.text.clone()))
                .collect(),
        })
        .map_err(|_| Error::Corrupt)
    }
    fn decode(json: &str, conversation: Uuid) -> Result<InputQueue, Error> {
        let value: Self = serde_json::from_str(json).map_err(|_| Error::Corrupt)?;
        if ![1, 2, 3, 4].contains(&value.version)
            || value.conversation != conversation
            || (value.version == 1 && !value.origins.is_empty())
            || (value.version < 4 && !value.paused_inputs.is_empty())
        {
            return Err(Error::Corrupt);
        }
        let mut user_inputs = std::collections::HashMap::new();
        for (id, input) in value.user_inputs {
            if value.version < 3
                || !value.inputs.iter().any(|(input_id, _)| *input_id == id)
                || user_inputs.insert(id, input).is_some()
            {
                return Err(Error::Corrupt);
            }
        }
        let mut origins = std::collections::HashMap::new();
        for (id, origin) in value.origins {
            if !value.inputs.iter().any(|(input_id, _)| *input_id == id)
                || origins.insert(id, origin).is_some()
            {
                return Err(Error::Corrupt);
            }
        }
        InputQueue::restore_with_paused_inputs(
            value.conversation,
            value.revision,
            value.paused,
            value
                .inputs
                .into_iter()
                .map(|(id, text)| QueuedInput {
                    user_input: user_inputs.remove(&id).unwrap_or_default(),
                    origin: origins.remove(&id),
                    id,
                    text,
                })
                .collect(),
            value.preparing,
            value.paused_inputs,
        )
        .map_err(|_| Error::Corrupt)
    }
}

/// Validate the immutable queue state stored with one admission. This does not
/// authorize starting a worker or assert that the current queue is unpaused.
pub(crate) fn validate_admission_snapshot(
    json: &str,
    conversation: Uuid,
    input_id: Uuid,
    expected_revision: u64,
) -> Result<(), Error> {
    let result = Snapshot::decode(json, conversation)?;
    let delta = result.revision().checked_sub(expected_revision);
    if !matches!(delta, Some(1 | 2))
        || result.preparing().is_some()
        || result.inputs().iter().any(|input| input.id == input_id)
        || (delta == Some(2) && result.paused())
    {
        return Err(Error::Corrupt);
    }
    Ok(())
}

/// Decode the immutable queue preview with the same contract as live queues.
pub(crate) fn decode_prepared_snapshot(
    json: &str,
    conversation: Uuid,
) -> Result<InputQueue, Error> {
    Snapshot::decode(json, conversation)
}

fn require_open(db: &Connection, conversation: Uuid) -> Result<(), Error> {
    edit::require_open_sql(db, conversation).map_err(|cause| cause.domain_or(Error::Storage))
}

fn load(db: &Connection, conversation: Uuid) -> Result<InputQueue, Error> {
    edit::load_sql(db, conversation).map_err(|cause| cause.domain_or(Error::Storage))
}

pub fn read_checked(
    db: &mut Connection,
    conversation: Uuid,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<InputQueue, Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    authorize(&tx)?;
    let queue = load(&tx, conversation)?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(queue)
}

#[cfg(test)]
pub fn apply_checked(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    edit: QueueEdit,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<InputQueue, Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let queue = apply_in_tx_sql(&tx, conversation, request, revision, edit, |db| {
        authorize(db).map_err(QueueSqlError::Domain)
    })
    .map_err(|cause| cause.domain_or(Error::Storage))?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(queue)
}

fn command_json(revision: u64, edit: &QueueEdit) -> String {
    use serde_json::json;
    let value = match edit {
        QueueEdit::Restore {
            input,
            source_id,
            index,
            paused,
        } => json!([
            "restore",
            input.id,
            source_id,
            index,
            input.text,
            input.origin,
            input.user_input,
            paused
        ]),
        QueueEdit::Add(input) if !input.user_input.is_empty() => {
            json!(["add", input.id, input.text, input.origin, input.user_input])
        }
        QueueEdit::Add(input) if input.origin.is_some() => {
            json!(["add", input.id, input.text, input.origin])
        }
        QueueEdit::Add(input) => json!(["add", input.id, input.text]),
        QueueEdit::Update {
            id,
            text,
            user_input,
        } if !user_input.is_empty() => json!(["update", id, text, user_input]),
        QueueEdit::Update { id, text, .. } => json!(["update", id, text]),
        QueueEdit::Remove(id) => json!(["remove", id]),
        QueueEdit::Reorder(ids) => json!(["reorder", ids]),
        QueueEdit::Pause => json!(["pause"]),
        QueueEdit::Resume => json!(["resume"]),
        QueueEdit::PauseInput(id) => json!(["pauseInput", id]),
        QueueEdit::ResumeInput(id) => json!(["resumeInput", id]),
        QueueEdit::Prepare(id) => json!(["prepare", id]),
        QueueEdit::Admitted(id) => json!(["admitted", id]),
        QueueEdit::Release(id) => json!(["release", id]),
    };
    json!([1, revision, value]).to_string()
}

#[cfg(test)]
#[path = "input_queue/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "input_queue/terminal_tests.rs"]
mod terminal_tests;

#[cfg(test)]
#[path = "input_queue/origin_tests.rs"]
mod origin_tests;
#[cfg(test)]
#[path = "input_queue/restore_tests.rs"]
mod restore_tests;
