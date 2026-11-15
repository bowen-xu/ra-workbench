use super::*;
use crate::agent::{
    writing_commands::PrepareRequest,
    writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
};
use crate::storage::AppStore;
use sha2::{Digest, Sha256};

pub(crate) const FAMILY: &str = "agent.writing.submission.prepare";

#[derive(Clone)]
pub(crate) struct WritingPrepareIntent {
    pub request: PrepareRequest,
    pub compact_revision: Option<u64>,
}
#[derive(Clone)]
pub(crate) struct WritingPrepareSource {
    pub store: AppStore,
    pub request: WritingSubmissionRequest,
    pub model: FrozenWritingModel,
}
pub(crate) enum WritingPrepareStatus {
    Committed {
        record: Box<SubmissionRecord>,
        receipt: Box<SqlImmediateReceipt>,
    },
    NotFound,
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl WritingPrepareIntent {
    pub(crate) fn operation(&self) -> Uuid {
        self.request.request_id
    }
    pub(crate) fn conversation(&self) -> Uuid {
        self.request.conversation_id
    }
    pub(crate) fn manuscript(&self) -> Uuid {
        self.request.context.manuscript_id
    }
}
fn sort(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.values_mut().for_each(sort);
            map.sort_keys();
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sort),
        _ => {}
    }
}
pub(super) fn identity(
    command: &WritingPrepareIntent,
) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    if [
        command.operation(),
        command.conversation(),
        command.manuscript(),
    ]
    .iter()
    .any(Uuid::is_nil)
        || command
            .compact_revision
            .is_some_and(|revision| revision > 9_007_199_254_740_991)
        || command.request.text.trim().is_empty()
        || command.request.text.len() > 1_048_576
        || (command.compact_revision.is_some() && command.request.text.trim() != "/compact")
    {
        return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput));
    }
    let purpose = match command.compact_revision {
        Some(expected) => serde_json::json!({"type":"compact","expectedRevision":expected}),
        None => serde_json::json!({"type":"regular"}),
    };
    let mut value =
        serde_json::json!({"schemaVersion":1,"request":command.request,"purpose":purpose});
    sort(&mut value);
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?;
    let mut hash = Sha256::new();
    hash.update(b"agent.writing.submission.prepare:1:");
    hash.update(bytes);
    Ok(ReceiptIdentity {
        operation_id: command.operation(),
        kind: FAMILY,
        version: 1,
        fingerprint: hash.finalize().into(),
    })
}
