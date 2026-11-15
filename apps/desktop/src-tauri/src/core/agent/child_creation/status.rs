//! Read a creation receipt using only the parent's recorded versioned intent.
use super::{Host, intent, note_tool_child_id, paper_tool_child_id, writing_tool_child_id};
use crate::agent::{
    catalog_commands::{
        ArchiveCommandError, ArchiveStatusResponse, CatalogCommandError, CreateBoundCommandError,
        CreateBoundStatusResponse,
    },
    repository::SqliteConversationCatalog,
    service::AgentSessionService,
    session_commands::SessionCommandError,
};
use research_agent_core::{
    conversation::{
        ArchiveConversation, ArchiveStatus, BindingKind, ConversationCatalog, ConversationId,
        CreateBoundStatus,
    },
    protocol::{FailureCategory, ItemKind},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub call_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub(crate) enum ErrorKind {
    InvalidInput,
    Active,
    IntentUnavailable,
    IdentityMismatch,
    Session(SessionCommandError),
    Catalog(CatalogCommandError),
    Creation(CreateBoundCommandError),
    Submission(ArchiveCommandError),
    Archive(ArchiveCommandError),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Error {
    conversation_id: Uuid,
    turn_id: Uuid,
    call_id: Uuid,
    #[serde(flatten)]
    pub(crate) kind: ErrorKind,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    schema_version: u8,
    conversation_id: Uuid,
    turn_id: Uuid,
    call_id: Uuid,
    watermark: u64,
    creation: CreateBoundStatusResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission: Option<SubmissionSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleanup: Option<ArchiveStatusResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionSnapshot {
    request_id: Uuid,
    epoch: Uuid,
    record: Option<SubmissionRecordIdentity>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionRecordIdentity {
    turn_id: Uuid,
    context_snapshot_id: Uuid,
}

fn cleanup_intent(
    failure: Option<&(String, FailureCategory)>,
    child: ConversationId,
) -> Result<Option<ArchiveConversation>, ErrorKind> {
    let Some((message, FailureCategory::Recovery)) = failure else {
        return Ok(None);
    };
    let Some(text) = message.strip_prefix("tool_outcome_unknown: ") else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    if value.get("childArchiveIntentVersion").is_none()
        || value
            .get("resultId")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            != Some(*child.as_uuid())
    {
        return Ok(None);
    }
    let invalid = || ErrorKind::IdentityMismatch;
    if value["childArchiveIntentVersion"].as_u64() != Some(1) {
        return Err(invalid());
    }
    let request = value["requestId"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|v| !v.is_nil())
        .ok_or_else(invalid)?;
    let operation = value["childArchiveOperationId"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|v| !v.is_nil())
        .ok_or_else(invalid)?;
    let watermark = value["childArchiveExpectedWatermark"]
        .as_u64()
        .filter(|v| *v <= 9_007_199_254_740_991)
        .ok_or_else(invalid)?;
    let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
    phase.extend_from_slice(request.as_bytes());
    if operation != Uuid::new_v5(child.as_uuid(), &phase) {
        return Err(invalid());
    }
    Ok(Some(ArchiveConversation {
        operation_id: operation,
        id: child,
        archived: true,
        expected_watermark: Some(watermark),
    }))
}

pub(crate) async fn execute(
    sessions: &AgentSessionService,
    catalog: &SqliteConversationCatalog,
    request: Request,
) -> Result<Response, Error> {
    let (conversation_id, turn_id, call_id) =
        (request.conversation_id, request.turn_id, request.call_id);
    execute_inner(sessions, catalog, request)
        .await
        .map_err(|kind| Error {
            conversation_id,
            turn_id,
            call_id,
            kind,
        })
}

async fn execute_inner(
    sessions: &AgentSessionService,
    catalog: &SqliteConversationCatalog,
    request: Request,
) -> Result<Response, ErrorKind> {
    if request.schema_version != 1
        || [request.conversation_id, request.turn_id, request.call_id]
            .iter()
            .any(Uuid::is_nil)
    {
        return Err(ErrorKind::InvalidInput);
    }
    let parent = ConversationId::from_uuid(request.conversation_id);
    // Do not call open(): status inspection must not recover or interrupt a turn.
    let history = sessions
        .read_projection(parent)
        .await
        .map_err(|e| ErrorKind::Session(e.into()))?;
    if history.active_turn().is_some() {
        return Err(ErrorKind::Active);
    }
    let turn = history
        .turn(request.turn_id)
        .ok_or(ErrorKind::IntentUnavailable)?;
    if turn.outcome.is_none() {
        return Err(ErrorKind::IntentUnavailable);
    }
    let provider_call = turn
        .items
        .values()
        .find_map(|item| match &item.kind {
            ItemKind::ToolCall {
                call_id,
                name,
                namespace: None,
                provider_call_id: Some(provider),
            } if *call_id == request.call_id
                && item.accepted
                && item.completed
                && name == "research_spawn_subtask" =>
            {
                Some(provider.as_str())
            }
            _ => None,
        })
        .ok_or(ErrorKind::IntentUnavailable)?;
    if turn.items.values().any(|item| {
        item.completed
            && matches!(item.kind,
        ItemKind::ToolResult { call_id, .. } if call_id == request.call_id)
    }) {
        return Err(ErrorKind::IntentUnavailable);
    }
    let original = turn
        .child_creation_intents
        .get(&request.call_id)
        .ok_or(ErrorKind::IntentUnavailable)?;
    let (host, child) = match original.binding.kind {
        BindingKind::Manuscript => (
            Host::Writing,
            writing_tool_child_id(request.turn_id, provider_call),
        ),
        BindingKind::Paper => (
            Host::Paper,
            paper_tool_child_id(request.turn_id, provider_call),
        ),
        BindingKind::Note | BindingKind::ReadingNote => (
            Host::Note,
            note_tool_child_id(request.turn_id, provider_call),
        ),
    };
    if *original != intent(child, original.binding.clone(), host) {
        return Err(ErrorKind::IdentityMismatch);
    }
    // A historical marker never substitutes for checking the current parent scope.
    catalog
        .read(parent, Some(original.binding.clone()))
        .await
        .map_err(|e| ErrorKind::Catalog(e.into()))?;
    let status = catalog
        .create_bound_status(original.clone())
        .await
        .map_err(|e| ErrorKind::Creation(e.into()))?;
    let submission = if matches!(&status, CreateBoundStatus::Committed(_)) {
        if let Some(request) = turn.child_submission_requests.get(&request.call_id) {
            catalog
                .read(original.id, Some(original.binding.clone()))
                .await
                .map_err(|e| ErrorKind::Catalog(e.into()))?;
            let snapshot = catalog
                .inspect_child_submission(parent, original.id, *request, original.binding.clone())
                .await
                .map_err(|e| ErrorKind::Submission(e.into()))?;
            if matches!(&status, CreateBoundStatus::Committed(proof) if proof.epoch != snapshot.epoch)
            {
                return Err(ErrorKind::IdentityMismatch);
            }
            Some(SubmissionSnapshot {
                request_id: *request,
                epoch: snapshot.epoch,
                record: snapshot.record.map(|(turn_id, context_snapshot_id)| {
                    SubmissionRecordIdentity {
                        turn_id,
                        context_snapshot_id,
                    }
                }),
            })
        } else {
            None
        }
    } else {
        None
    };
    let recorded = if matches!(&status, CreateBoundStatus::Committed(_)) {
        catalog
            .inspect_child_cleanup(original.id)
            .await
            .map_err(|e| ErrorKind::Archive(e.into()))?
    } else {
        None
    };
    if let (Some(saved), Some(request)) = (
        &recorded,
        turn.child_submission_requests.get(&request.call_id),
    ) && saved.intent.request_id != *request
    {
        return Err(ErrorKind::IdentityMismatch);
    }
    let diagnostic = cleanup_intent(turn.failure.as_ref(), original.id)?;
    if let (Some(saved), Some(reported)) = (&recorded, &diagnostic)
        && saved.intent.archive != *reported
    {
        return Err(ErrorKind::IdentityMismatch);
    }
    let original_cleanup = recorded
        .map(|saved| (saved.intent.archive, Some(saved.epoch)))
        .or_else(|| diagnostic.map(|command| (command, None)));
    if let (Some((command, _)), Some(request)) = (
        &original_cleanup,
        turn.child_submission_requests.get(&request.call_id),
    ) {
        let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
        phase.extend_from_slice(request.as_bytes());
        if command.operation_id != Uuid::new_v5(original.id.as_uuid(), &phase) {
            return Err(ErrorKind::IdentityMismatch);
        }
    }
    let cleanup = if let Some((command, expected_epoch)) = original_cleanup {
        catalog
            .read(original.id, Some(original.binding.clone()))
            .await
            .map_err(|e| ErrorKind::Catalog(e.into()))?;
        let status = catalog
            .archive_status(command.clone())
            .await
            .map_err(|e| ErrorKind::Archive(e.into()))?;
        let epoch = match &status {
            ArchiveStatus::Committed(proof) => proof.epoch,
            ArchiveStatus::NoSqlReceipt { epoch, .. }
            | ArchiveStatus::Unconfirmed { epoch, .. } => *epoch,
        };
        if expected_epoch.is_some_and(|expected| expected != epoch) {
            return Err(ErrorKind::IdentityMismatch);
        }
        Some(ArchiveStatusResponse::new(
            command.operation_id,
            *command.id.as_uuid(),
            status,
            "agent.conversation.archive",
        ))
    } else {
        None
    };
    Ok(Response {
        schema_version: 1,
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
        call_id: request.call_id,
        watermark: history.seq(),
        creation: CreateBoundStatusResponse::new(
            original.operation_id,
            *original.id.as_uuid(),
            status,
            "agent.conversation.create_bound",
        ),
        submission,
        cleanup,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_child_creation_status(
    sessions: State<'_, Arc<AgentSessionService>>,
    catalog: State<'_, SqliteConversationCatalog>,
    request: Request,
) -> Result<Response, Error> {
    execute(sessions.inner(), catalog.inner(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cleanup_intent_is_bound_to_the_original_child_and_version() {
        let child = Uuid::new_v4();
        let request = Uuid::new_v4();
        let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
        phase.extend_from_slice(request.as_bytes());
        let operation = Uuid::new_v5(&child, &phase);
        let detail = json!({"resultId":child,"requestId":request,"childArchiveIntentVersion":1,
            "childArchiveOperationId":operation,"childArchiveExpectedWatermark":7});
        let make = |v: serde_json::Value| {
            (
                format!("tool_outcome_unknown: {v}"),
                FailureCategory::Recovery,
            )
        };
        let command = cleanup_intent(
            Some(&make(detail.clone())),
            ConversationId::from_uuid(child),
        )
        .unwrap()
        .unwrap();
        assert_eq!(command.operation_id, operation);
        assert_eq!(command.expected_watermark, Some(7));
        assert!(
            cleanup_intent(
                Some(&make(detail.clone())),
                ConversationId::from_uuid(Uuid::new_v4())
            )
            .unwrap()
            .is_none()
        );
        for (key, bad) in [
            ("childArchiveIntentVersion", json!(2)),
            ("childArchiveOperationId", json!(Uuid::new_v4())),
            (
                "childArchiveExpectedWatermark",
                json!(9_007_199_254_740_992u64),
            ),
        ] {
            let mut modified = detail.clone();
            modified[key] = bad;
            assert!(matches!(
                cleanup_intent(Some(&make(modified)), ConversationId::from_uuid(child)),
                Err(ErrorKind::IdentityMismatch)
            ));
        }
        let mut legacy = detail;
        legacy
            .as_object_mut()
            .unwrap()
            .remove("childArchiveIntentVersion");
        assert!(
            cleanup_intent(Some(&make(legacy)), ConversationId::from_uuid(child))
                .unwrap()
                .is_none()
        );
    }
}
