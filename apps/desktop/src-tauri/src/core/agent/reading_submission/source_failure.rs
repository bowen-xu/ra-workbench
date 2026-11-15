//! Finite source diagnostics beside the original paper error and request identity.
use super::{Error, PrepareRequest};
use crate::runtime_plugins::literature_hbs::NativeFailure;
use serde::{Serialize, Serializer, ser::Error as _};
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SourceFailure {
    original: Box<Error>,
    message: String,
    diagnostic: Box<NativeFailure>,
    code_override: Option<String>,
    conversation_id: Uuid,
    request_id: Uuid,
    paper_id: String,
    paper_context_snapshot_id: String,
}
impl SourceFailure {
    pub(crate) fn attach(
        original: Error,
        message: String,
        diagnostic: Option<NativeFailure>,
        code_override: Option<String>,
        request: &PrepareRequest,
    ) -> Error {
        let Some(diagnostic) = diagnostic else {
            return original;
        };
        Error::Source(Box::new(Self {
            original: Box::new(original),
            message,
            diagnostic: Box::new(diagnostic),
            code_override,
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            paper_id: request.paper_id.clone(),
            paper_context_snapshot_id: request.paper_context_snapshot_id.clone(),
        }))
    }
    pub(super) fn code(&self) -> &str {
        self.code_override
            .as_deref()
            .unwrap_or_else(|| self.original.code())
    }
}
impl Serialize for SourceFailure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let original = serde_json::to_value(&self.original).map_err(S::Error::custom)?;
        // Preserve the original storage reason and unknown-result payload.
        let mut fields = match original {
            Value::Object(fields) => fields,
            Value::String(_) => Map::new(),
            _ => return Err(S::Error::custom("invalid paper error representation")),
        };
        fields.insert("code".into(), json!(self.code()));
        fields.insert("message".into(), json!(self.message));
        fields.insert(
            "nativeFailure".into(),
            serde_json::to_value(&self.diagnostic).map_err(S::Error::custom)?,
        );
        fields.insert("paperContextOperation".into(), json!("submission"));
        fields.insert("conversationId".into(), json!(self.conversation_id));
        fields.insert("requestId".into(), json!(self.request_id));
        fields.insert("paperId".into(), json!(self.paper_id));
        fields.insert(
            "paperContextSnapshotId".into(),
            json!(self.paper_context_snapshot_id),
        );
        fields.serialize(serializer)
    }
}
