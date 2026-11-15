//! One child creation phase keeps its complete identity through an unknown
//! storage result. This does not deduplicate the surrounding spawn RPC.
use super::catalog_commands::CreateBoundCommandError;
use research_agent_core::conversation::{
    ConversationBinding, ConversationCatalog, ConversationId, CreateBoundConversation,
    CreateBoundProof,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[path = "child_creation/status.rs"]
pub(crate) mod status;

#[derive(Clone, Copy)]
pub(crate) enum Host {
    Writing,
    Paper,
    Note,
}

/// The provider call id is durably recorded in the accepted tool item before
/// execution. A tool child can therefore be identified after a lost reply.
pub(crate) fn writing_tool_child_id(turn_id: Uuid, call_id: &str) -> Uuid {
    let mut name = b"agent.writing_child.tool_call:v1:".to_vec();
    name.extend_from_slice(call_id.as_bytes());
    Uuid::new_v5(&turn_id, &name)
}

pub(crate) fn paper_tool_child_id(turn_id: Uuid, call_id: &str) -> Uuid {
    tool_child_id(turn_id, call_id, b"agent.reading_child.tool_call:v1:")
}

pub(crate) fn note_tool_child_id(turn_id: Uuid, call_id: &str) -> Uuid {
    tool_child_id(turn_id, call_id, b"agent.note_child.tool_call:v1:")
}

fn tool_child_id(turn_id: Uuid, call_id: &str, prefix: &[u8]) -> Uuid {
    let mut name = prefix.to_vec();
    name.extend_from_slice(call_id.as_bytes());
    Uuid::new_v5(&turn_id, &name)
}

pub(crate) fn intent(
    id: Uuid,
    binding: ConversationBinding,
    host: Host,
) -> CreateBoundConversation {
    let phase: &[u8] = match host {
        Host::Writing => b"agent.writing_child.create_bound:v1",
        Host::Paper => b"agent.reading_child.create_bound:v1",
        Host::Note => b"agent.note_child.create_bound:v1",
    };
    CreateBoundConversation {
        operation_id: Uuid::new_v5(&id, phase),
        id: ConversationId::from_uuid(id),
        binding,
    }
}

/// The caller already owns these resource identifiers. This recovery intent is
/// separate from storage diagnostics and contains no prompt or frozen content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreationIntent {
    pub operation_id: Uuid,
    pub id: Uuid,
    pub binding: ConversationBinding,
}
impl From<&CreateBoundConversation> for CreationIntent {
    fn from(command: &CreateBoundConversation) -> Self {
        Self {
            operation_id: command.operation_id,
            id: *command.id.as_uuid(),
            binding: command.binding.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationFailure {
    pub creation: CreationIntent,
    pub reason: CreateBoundCommandError,
}

/// Preserve only verified native identities. The binding itself remains owned
/// by the parent's frozen scope; do not put arbitrary resource strings in a
/// user-visible diagnostic or infer a new binding from a later head.
#[cfg(test)]
pub(crate) fn unknown_tool_error(
    failure: &CreationFailure,
    expected: &CreateBoundConversation,
    parent: Uuid,
    turn: Uuid,
    context: Uuid,
) -> Option<research_agent_core::runner::RunError> {
    use research_agent_core::{
        conversation::CatalogDataFailureKind,
        runner::{RunError, ToolOutcomeDiagnostic},
    };
    let CreateBoundCommandError::Data(data) = &failure.reason else {
        return None;
    };
    if !matches!(
        data.kind,
        CatalogDataFailureKind::Unknown
            | CatalogDataFailureKind::Partial
            | CatalogDataFailureKind::ExternalUnknown
            | CatalogDataFailureKind::RecoveryRequired
    ) {
        return None;
    }
    if failure.creation != expected.into() || data.diagnostic.operation_id != expected.operation_id
    {
        return Some(RunError::ToolOutcomeUnknown);
    }
    let detail = serde_json::json!({
        "code":"CHILD_CREATION_UNCONFIRMED", "phase":data.diagnostic.phase,
        "safeCause":data.diagnostic.safe_cause, "conversationId":parent, "turnId":turn,
        "contextSnapshotId":context, "requestId":expected.operation_id,
        "resultId":expected.id.as_uuid(), "sqliteCode":data.diagnostic.sqlite_code,
        "sqliteExtendedCode":data.diagnostic.sqlite_extended_code,
    });
    Some(
        ToolOutcomeDiagnostic::try_from_value(detail)
            .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
            .unwrap_or(RunError::ToolOutcomeUnknown),
    )
}

pub(crate) async fn create(
    catalog: &impl ConversationCatalog,
    command: &CreateBoundConversation,
) -> Result<CreateBoundProof, CreationFailure> {
    catalog
        .create_bound(command.clone())
        .await
        .map_err(|error| CreationFailure {
            creation: command.into(),
            reason: error.into(),
        })
}

#[cfg(test)]
#[path = "child_creation/tests.rs"]
mod tests;
