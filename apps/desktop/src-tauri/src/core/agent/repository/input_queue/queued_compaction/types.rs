use super::*;
use crate::{
    agent::writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
    storage::AppStore,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(crate) struct PrepareIntent {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub expected_revision: u64,
    pub previous_request_id: Option<Uuid>,
    pub request_json: Value,
}
#[derive(Clone)]
pub(crate) struct PrepareSource {
    pub store: AppStore,
    pub request: WritingSubmissionRequest,
    pub model: FrozenWritingModel,
}
#[derive(Clone)]
pub(crate) struct ReadIntent {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SettleIntent {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub claim_revision: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct QueuedCompactionState {
    pub record: SubmissionRecord,
    pub queue: InputQueue,
    pub claim_revision: u64,
    pub expected_revision: u64,
    pub outcome: Option<TurnOutcome>,
    pub current: bool,
    pub retirement: Option<Box<crate::data_interface::QueuePreviewRetirement>>,
}
#[derive(Debug)]
pub(crate) enum QueuedCompactionRead {
    Compact(QueuedCompactionState),
    Submission {
        record: SubmissionRecord,
        queue: InputQueue,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct SettleState {
    pub queue: InputQueue,
    pub outcome: TurnOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct PrepareRecord {
    pub kind: String,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub expected_revision: u64,
    pub previous_request_id: Option<Uuid>,
    pub request_sha256: String,
}
pub(crate) fn prepare_command(intent: &PrepareIntent) -> Result<String, SqlMutationError> {
    ids(&[
        intent.conversation_id,
        intent.manuscript_id,
        intent.input_id,
        intent.request_id,
    ])?;
    revision(intent.expected_queue_revision, true)?;
    revision(intent.expected_revision, false)?;
    if intent
        .previous_request_id
        .is_some_and(|id| id.is_nil() || id == intent.request_id)
    {
        return Err(CatalogError::InvalidInput.into());
    }
    let mut request = intent.request_json.clone();
    sort(&mut request);
    let bytes = serde_json::to_vec(&request).map_err(|_| CatalogError::InvalidInput)?;
    serde_json::to_string(&PrepareRecord {
        kind: "queuedCompactionPrepare:v1".into(),
        conversation_id: intent.conversation_id,
        manuscript_id: intent.manuscript_id,
        input_id: intent.input_id,
        request_id: intent.request_id,
        expected_queue_revision: intent.expected_queue_revision,
        expected_revision: intent.expected_revision,
        previous_request_id: intent.previous_request_id,
        request_sha256: format!("{:x}", Sha256::digest(bytes)),
    })
    .map_err(|_| CatalogError::InvalidInput.into())
}
pub(crate) fn settle_command(intent: &SettleIntent) -> Result<String, SqlMutationError> {
    ids(&[
        intent.operation_id,
        intent.conversation_id,
        intent.manuscript_id,
        intent.input_id,
        intent.request_id,
    ])?;
    revision(intent.expected_queue_revision, true)?;
    revision(intent.claim_revision, true)?;
    serde_json::to_string(&serde_json::json!({"kind":"queuedCompactionSettle:v1","intent":intent}))
        .map_err(|_| CatalogError::InvalidInput.into())
}
pub(super) fn ids(values: &[Uuid]) -> Result<(), SqlMutationError> {
    if values.iter().any(Uuid::is_nil) {
        Err(CatalogError::InvalidInput.into())
    } else {
        Ok(())
    }
}
fn revision(value: u64, increment: bool) -> Result<(), SqlMutationError> {
    if value > 9_007_199_254_740_991 || increment && value == 9_007_199_254_740_991 {
        Err(CatalogError::InvalidInput.into())
    } else {
        Ok(())
    }
}
fn sort(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.values_mut().for_each(sort);
            map.sort_keys();
        }
        Value::Array(items) => items.iter_mut().for_each(sort),
        _ => {}
    }
}
