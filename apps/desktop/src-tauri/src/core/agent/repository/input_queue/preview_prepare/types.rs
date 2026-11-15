use super::*;
use crate::{
    agent::writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
    storage::AppStore,
};
use research_agent_core::submission::SubmissionRecord;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(crate) struct QueuePreviewPrepareIntent {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub request_json: Value,
}
#[derive(Clone)]
pub(crate) struct QueuePreviewPrepareSource {
    pub store: AppStore,
    pub request: WritingSubmissionRequest,
    pub model: FrozenWritingModel,
}
#[derive(Clone, Debug)]
pub(crate) struct QueuePreviewPrepared {
    pub record: SubmissionRecord,
    pub queue: InputQueue,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenPrepareCommand {
    pub kind: String,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub request_sha256: String,
}
impl FrozenPrepareCommand {
    pub(crate) fn decode(encoded: &str) -> Result<Self, SqlMutationError> {
        let value: Self = serde_json::from_str(encoded).map_err(|_| CatalogError::Corrupt)?;
        if value.kind != "queuePreviewPrepare:v1"
            || [
                value.conversation_id,
                value.manuscript_id,
                value.input_id,
                value.request_id,
            ]
            .iter()
            .any(Uuid::is_nil)
            || value.expected_queue_revision >= 9_007_199_254_740_991
            || value.request_sha256.len() != 64
            || !value
                .request_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || serde_json::to_string(&value).map_err(|_| CatalogError::Corrupt)? != encoded
        {
            return Err(CatalogError::Corrupt.into());
        }
        Ok(value)
    }
}
pub(crate) fn command_fingerprint(encoded: &str) -> [u8; 32] {
    Sha256::digest(format!("agent.queue.preview.prepare:1:{encoded}")).into()
}

pub(crate) fn command(intent: &QueuePreviewPrepareIntent) -> Result<String, SqlMutationError> {
    if [
        intent.conversation_id,
        intent.manuscript_id,
        intent.input_id,
        intent.request_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        || intent.expected_queue_revision >= 9_007_199_254_740_991
        || !intent.request_json.is_object()
    {
        return Err(CatalogError::InvalidInput.into());
    }
    let mut request = intent.request_json.clone();
    sort(&mut request);
    let bytes = serde_json::to_vec(&request).map_err(|_| CatalogError::InvalidInput)?;
    serde_json::to_string(&FrozenPrepareCommand {
        kind: "queuePreviewPrepare:v1".into(),
        conversation_id: intent.conversation_id,
        manuscript_id: intent.manuscript_id,
        input_id: intent.input_id,
        request_id: intent.request_id,
        expected_queue_revision: intent.expected_queue_revision,
        request_sha256: format!("{:x}", Sha256::digest(bytes)),
    })
    .map_err(|_| CatalogError::InvalidInput.into())
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
