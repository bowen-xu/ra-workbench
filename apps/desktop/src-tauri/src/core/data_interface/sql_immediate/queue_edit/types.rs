use super::*;
use research_agent_core::input_queue::QueueEdit;
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use sha2::{Digest, Sha256};

pub(crate) const QUEUE_EDIT_FAMILY: &str = "agent.input_queue.edit";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum QueueEditScope {
    Context,
    Manuscript { manuscript_id: Uuid },
    Paper { paper_id: String },
    Note { resource_id: Uuid },
    ReadingNote { resource_id: Uuid },
}

#[derive(Clone, Debug)]
pub(crate) struct QueueEditIntent {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub expected_revision: u64,
    pub expected_epoch: Option<Uuid>,
    pub scope: QueueEditScope,
    pub edit: QueueEdit,
}

/// Preserve every original wire field, before host provenance or Restore lowering.
fn wire_edit(edit: &QueueEdit) -> Option<serde_json::Value> {
    use serde_json::json;
    Some(match edit {
        QueueEdit::Add(input) => {
            json!({"type":"add","inputId":input.id,"text":input.text,"userInput":input.user_input})
        }
        QueueEdit::Restore {
            input,
            source_id,
            index,
            ..
        } => {
            json!({"type":"restore","inputId":input.id,"sourceInputId":source_id,"index":index,"text":input.text,"userInput":input.user_input})
        }
        QueueEdit::Update {
            id,
            text,
            user_input,
        } => json!({"type":"update","inputId":id,"text":text,"userInput":user_input}),
        QueueEdit::Remove(id) => json!({"type":"remove","inputId":id}),
        QueueEdit::Reorder(ids) => json!({"type":"reorder","inputIds":ids}),
        QueueEdit::Pause => json!({"type":"pause"}),
        QueueEdit::Resume => json!({"type":"resume"}),
        QueueEdit::PauseInput(id) => json!({"type":"pauseInput","inputId":id}),
        QueueEdit::ResumeInput(id) => json!({"type":"resumeInput","inputId":id}),
        QueueEdit::Prepare(_) | QueueEdit::Release(_) | QueueEdit::Admitted(_) => return None,
    })
}
impl Serialize for QueueEditIntent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            "QueueEditIntent",
            if self.expected_epoch.is_some() { 6 } else { 5 },
        )?;
        state.serialize_field("conversationId", &self.conversation_id)?;
        state.serialize_field("requestId", &self.request_id)?;
        state.serialize_field("expectedRevision", &self.expected_revision)?;
        if let Some(epoch) = self.expected_epoch {
            state.serialize_field("expectedEpoch", &epoch)?;
        }
        state.serialize_field("scope", &self.scope)?;
        state.serialize_field("edit", &wire_edit(&self.edit))?;
        state.end()
    }
}
impl QueueEditIntent {
    pub(super) fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        let valid_scope = match &self.scope {
            QueueEditScope::Context => true,
            QueueEditScope::Manuscript { manuscript_id } => !manuscript_id.is_nil(),
            QueueEditScope::Note { resource_id } | QueueEditScope::ReadingNote { resource_id } => {
                !resource_id.is_nil()
            }
            QueueEditScope::Paper { paper_id } => {
                !paper_id.trim().is_empty() && paper_id.len() <= 256 && !paper_id.contains('\0')
            }
        };
        if self.conversation_id.is_nil()
            || self.request_id.is_nil()
            || self.expected_epoch.is_some_and(|epoch| epoch.is_nil())
            || !valid_scope
            || self.expected_revision >= 9_007_199_254_740_991
            || wire_edit(&self.edit).is_none()
        {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    self.request_id,
                    None,
                    "validate",
                    "invalid_queue_edit_identity",
                ),
            )));
        }
        let bytes = serde_json::to_vec(&(QUEUE_EDIT_FAMILY, 1u32, self)).map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                self.request_id,
                None,
                "validate",
                "queue_edit_encoding_failed",
            )))
        })?;
        Ok(ReceiptIdentity {
            operation_id: self.request_id,
            kind: QUEUE_EDIT_FAMILY,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
    pub(super) fn lowered(&self) -> QueueEdit {
        use research_agent_core::conversation::{BindingKind, ConversationBinding};
        let mut edit = self.edit.clone();
        if let QueueEdit::Add(input) = &mut edit {
            input.origin = match &self.scope {
                QueueEditScope::Context => None,
                QueueEditScope::Manuscript { manuscript_id } => {
                    Some((BindingKind::Manuscript, manuscript_id.to_string()))
                }
                QueueEditScope::Paper { paper_id } => Some((BindingKind::Paper, paper_id.clone())),
                QueueEditScope::Note { resource_id } => {
                    Some((BindingKind::Note, resource_id.to_string()))
                }
                QueueEditScope::ReadingNote { resource_id } => {
                    Some((BindingKind::ReadingNote, resource_id.to_string()))
                }
            }
            .map(|(kind, resource_id)| ConversationBinding { kind, resource_id });
        }
        edit
    }
}

#[derive(Debug)]
pub(crate) enum QueueEditOutcome {
    Committed {
        queue: InputQueue,
        receipt: SqlImmediateReceipt,
    },
    LegacyReplay(InputQueue),
}
impl QueueEditOutcome {
    pub(crate) fn into_queue(self) -> InputQueue {
        match self {
            Self::Committed { queue, receipt } => {
                debug_assert!(!receipt.operation_id.is_nil());
                queue
            }
            Self::LegacyReplay(queue) => queue,
        }
    }
}
#[derive(Debug)]
pub(crate) enum QueueEditStatus {
    Committed {
        queue: InputQueue,
        receipt: SqlImmediateReceipt,
    },
    Deleted {
        receipt: SqlImmediateReceipt,
        detail: Box<SqlImmediateDiagnostic>,
    },
    NotFound,
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum QueueEditFailure {
    #[error(transparent)]
    Domain(input_queue::Error),
    #[error(transparent)]
    Storage(#[from] SqlImmediateFailure),
}
