//! Read frozen queue admission and execution facts without taking a writer lease.
use super::*;
use crate::agent::{
    execution_profile::{AgentExecutionMode, execution_profile},
    repository::submissions,
    writing_submission,
};
use research_agent_core::{
    protocol::{ModelIdentity, TaskKind, TurnOutcome},
    submission::SubmissionRecord,
};
use research_data_runtime::ReadFence;

#[derive(Clone)]
pub(crate) struct QueueStartIntent {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
}
pub(crate) enum Fact {
    Prepared(Box<SubmissionRecord>),
    Admitted(Box<SubmissionRecord>),
    Started {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
    },
    Missing,
}
pub(crate) struct InspectionError {
    pub safe_cause: &'static str,
    pub sql: Option<rusqlite::Error>,
}
impl InspectionError {
    fn sql(safe_cause: &'static str, error: rusqlite::Error) -> Self {
        Self {
            safe_cause,
            sql: Some(error),
        }
    }
}
impl From<&'static str> for InspectionError {
    fn from(safe_cause: &'static str) -> Self {
        Self {
            safe_cause,
            sql: None,
        }
    }
}
type Inspection<T> = Result<T, InspectionError>;

pub(crate) fn inspect(
    db: &Connection,
    intent: &QueueStartIntent,
    fence: ReadFence,
) -> Inspection<Fact> {
    let c = intent.conversation_id.to_string();
    let r = intent.request_id.to_string();
    let i = intent.input_id.to_string();
    let record = submissions::read(db, intent.conversation_id, intent.request_id)
        .map_err(|_| "queue_start_submission_invalid")?;
    let mut statement = db.prepare("select input_id,conversation_id,request_id,expected_revision,result_json from agent_input_queue_admissions where input_id=?1 or (conversation_id=?2 and request_id=?3)").map_err(|error| InspectionError::sql("queue_start_admission_unavailable", error))?;
    let rows = statement
        .query_map(params![i, c, r], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| InspectionError::sql("queue_start_admission_unavailable", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| InspectionError::sql("queue_start_admission_invalid", error))?;
    let Some(record) = record else {
        let evidence: bool = db.query_row("select exists(select 1 from agent_queue_submission_origins where request_id=?1 or input_id=?2) or exists(select 1 from agent_input_queue_previews where request_id=?1 or input_id=?2) or exists(select 1 from agent_items where conversation_id=?3 and json_extract(payload_json,'$.requestId')=?1) or exists(select 1 from agent_input_queues where conversation_id=?3 and json_extract(snapshot_json,'$.preparing')=?2)",params![r,i,c],|row|row.get(0)).map_err(|error| InspectionError::sql("queue_start_origin_unavailable", error))?;
        return if rows.is_empty() && !evidence {
            Ok(Fact::Missing)
        } else {
            Err("queue_start_submission_missing".into())
        };
    };
    if record.context_snapshot_id != intent.context_snapshot_id
        || record.input.model != intent.model
    {
        return Err("queue_start_intent_mismatch".into());
    }
    writing_submission::validate_scope(&record, intent.manuscript_id)
        .map_err(|_| "queue_start_scope_mismatch")?;
    if writing_submission::read_compaction_revision_in(
        db,
        intent.conversation_id,
        intent.request_id,
    )
    .map_err(|_| "queue_start_purpose_invalid")?
    .is_some()
    {
        return Err("queue_start_purpose_mismatch".into());
    }
    let origin: Option<String> = db.query_row("select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2",params![c,r],|row|row.get(0)).optional().map_err(|error| InspectionError::sql("queue_start_origin_unavailable", error))?;
    if origin.as_deref() != Some(i.as_str()) {
        return Err("queue_start_origin_mismatch".into());
    }
    let started = history(db, &record)?;
    // A real historical execution remains observable after a model/profile is
    // retired. This endpoint does not prove the original mode or grant a new
    // execution; only unstarted facts consult the current profile policy.
    if started.is_none() {
        let profile = execution_profile(&intent.model, intent.mode)
            .map_err(|_| "queue_start_mode_mismatch")?;
        if record
            .input
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| !profile.capabilities.reasoning_efforts.contains(effort))
        {
            return Err("queue_start_mode_mismatch".into());
        }
    }
    if rows.len() > 1 {
        return Err("queue_start_admission_ambiguous".into());
    }
    if let Some((input, conversation, request, revision, json)) = rows.first() {
        if input != &i
            || conversation != &c
            || request != &r
            || *revision != intent.expected_revision
        {
            return Err("queue_start_admission_mismatch".into());
        }
        validate_admission_snapshot(json, intent.conversation_id, intent.input_id, *revision)
            .map_err(|_| "queue_start_admission_snapshot_invalid")?;
        return Ok(match started {
            Some(outcome) => Fact::Started {
                turn_id: record.turn_id,
                outcome,
            },
            None => Fact::Admitted(Box::new(record)),
        });
    }
    if started.is_some() {
        return Err("queue_start_admission_missing".into());
    }
    let pointer: Option<String> = db.query_row("select request_id from agent_input_queue_previews where conversation_id=?1 and input_id=?2",params![c,i],|row|row.get(0)).optional().map_err(|error| InspectionError::sql("queue_start_preview_unavailable", error))?;
    let current =
        load(db, intent.conversation_id).map_err(|_| "queue_start_current_queue_invalid")?;
    if pointer.as_deref() != Some(r.as_str())
        || current.preparing() != Some(intent.input_id)
        || !current
            .inputs()
            .iter()
            .any(|input| input.id == intent.input_id && input.text == record.input.text)
    {
        return Err("queue_start_original_claim_absent".into());
    }
    if intent.expected_revision > current.revision()
        || !typed_preparation(db, intent, fence, &current)?
    {
        return Err("queue_start_legacy_preparation_unconfirmed".into());
    }
    Ok(Fact::Prepared(Box::new(record)))
}

fn history(db: &Connection, record: &SubmissionRecord) -> Inspection<Option<Option<TurnOutcome>>> {
    use submissions::start_observation::{self, Error};
    start_observation::observe(db, record, TaskKind::Regular).map_err(|error| match error {
        Error::Unavailable(error) => InspectionError::sql("queue_start_history_unavailable", error),
        Error::InvalidRow(error) => InspectionError::sql("queue_start_history_invalid", error),
        Error::Invalid => "queue_start_history_invalid".into(),
        Error::IdentityMismatch => "queue_start_history_identity_mismatch".into(),
        Error::PrefixInvalid => "queue_start_history_prefix_invalid".into(),
    })
}

fn typed_preparation(
    db: &Connection,
    intent: &QueueStartIntent,
    fence: ReadFence,
    current: &InputQueue,
) -> Inspection<bool> {
    type StoredPrepareProof = (String, String, String, u32, String, u64, Vec<u8>, String);
    let row: Option<StoredPrepareProof> = db.query_row("select p.conversation_id,p.command_json,p.result_json,s.version,s.epoch,s.sql_fence_position,s.fingerprint,s.result_json from agent_input_queue_receipts p join data_sql_operation_receipts s on s.operation_id=p.request_id and s.kind='agent.queue.preview.prepare' where p.request_id=?1",[intent.request_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?))).optional().map_err(|error| InspectionError::sql("queue_start_prepare_proof_unavailable", error))?;
    let Some((conversation, command, result, version, epoch, position, hash, proof_result)) = row
    else {
        return Ok(false);
    };
    let fingerprint = preview_prepare::command_fingerprint(&command);
    let command = preview_prepare::FrozenPrepareCommand::decode(&command)
        .map_err(|_| "queue_start_prepare_proof_invalid")?;
    let queue = Snapshot::decode(&result, intent.conversation_id)
        .map_err(|_| "queue_start_prepare_proof_invalid")?;
    let original = queue
        .inputs()
        .iter()
        .find(|input| input.id == intent.input_id);
    let head = current
        .inputs()
        .iter()
        .find(|input| input.id == intent.input_id);
    if conversation != intent.conversation_id.to_string()
        || command.conversation_id != intent.conversation_id
        || command.manuscript_id != intent.manuscript_id
        || command.input_id != intent.input_id
        || command.request_id != intent.request_id
        || command.expected_queue_revision.checked_add(1) != Some(queue.revision())
        || queue.revision() > intent.expected_revision
        || queue.preparing() != Some(intent.input_id)
        || original.is_none()
        || original != head
        || version != 1
        || epoch != fence.epoch.to_string()
        || position > fence.position
        || hash.as_slice() != fingerprint
        || proof_result != "null"
    {
        return Err("queue_start_prepare_proof_invalid".into());
    }
    Ok(true)
}
