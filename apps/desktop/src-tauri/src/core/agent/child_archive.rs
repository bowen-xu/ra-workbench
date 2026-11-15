//! Cleanup belongs to the original child submission, never its current head.
use super::{catalog_commands::ArchiveCommandError, repository::SqliteConversationCatalog};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{ArchiveConversation, CatalogError, ConversationCatalog, ConversationId},
    protocol::{AgentEvent, EventPayload},
};
use rusqlite::Connection;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveIntent {
    operation_id: Uuid,
    id: Uuid,
    archived: bool,
    expected_watermark: Option<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFailure {
    pub request_id: Uuid,
    pub archive: Option<ArchiveIntent>,
    pub reason: ArchiveCommandError,
}

/// The preview replaces only the failed external cleanup operation. The real
/// runner, creation owner, history and receipt lookup remain in use.
#[cfg(all(test, debug_assertions, feature = "development-data"))]
fn preview_unconfirmed(
    command: &ArchiveConversation,
) -> research_agent_core::conversation::CatalogMutationError {
    use research_agent_core::conversation::{
        CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic,
    };
    research_agent_core::conversation::CatalogMutationError::Data(CatalogDataFailure {
        kind: CatalogDataFailureKind::Unknown,
        diagnostic: Box::new(CatalogMutationDiagnostic {
            operation_id: command.operation_id,
            epoch: None,
            sql_fence: None,
            phase: "preview-child-cleanup",
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause: "synthetic_preview_cleanup_unconfirmed",
        }),
    })
}

/// A failed cleanup leaves the created child unresolved, even when the archive
/// itself was rejected before committing. Do not return it as a retryable tool
/// result: a subsequent model call would allocate a different child.
pub(crate) fn unresolved_tool_error(
    failure: &ArchiveFailure,
    child: Uuid,
) -> research_agent_core::runner::RunError {
    use research_agent_core::runner::{RunError, ToolOutcomeDiagnostic};
    let mut detail = serde_json::json!({
        "code": "CHILD_CLEANUP_UNCONFIRMED",
        "phase": "child-cleanup",
        "safeCause": "child_archive_unconfirmed",
        "requestId": failure.request_id,
        "resultId": child,
    });
    if let Some(intent) = &failure.archive {
        let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
        phase.extend_from_slice(failure.request_id.as_bytes());
        if intent.id != child
            || !intent.archived
            || intent.expected_watermark.is_none()
            || intent.operation_id != Uuid::new_v5(&child, &phase)
        {
            return RunError::ToolOutcomeUnknown;
        }
        detail["journalRequestId"] = serde_json::json!(intent.operation_id.to_string());
        detail["childArchiveIntentVersion"] = serde_json::json!(1);
        detail["childArchiveOperationId"] = serde_json::json!(intent.operation_id);
        detail["childArchiveExpectedWatermark"] = serde_json::json!(intent.expected_watermark);
        if let ArchiveCommandError::Data(data) = &failure.reason {
            if data.diagnostic.operation_id != intent.operation_id {
                return RunError::ToolOutcomeUnknown;
            }
            detail["code"] = serde_json::json!(data.kind.code());
            detail["phase"] = serde_json::json!(data.diagnostic.phase);
            detail["safeCause"] = serde_json::json!(data.diagnostic.safe_cause);
            detail["sqliteCode"] = serde_json::json!(data.diagnostic.sqlite_code);
            detail["sqliteExtendedCode"] = serde_json::json!(data.diagnostic.sqlite_extended_code);
        }
    } else if let ArchiveCommandError::Data(data) = &failure.reason {
        let prepare =
            crate::data_interface::ChildCleanupIntent::preparation_id(child, failure.request_id);
        if data.diagnostic.operation_id != prepare {
            return RunError::ToolOutcomeUnknown;
        }
        detail["journalRequestId"] = serde_json::json!(prepare.to_string());
        detail["code"] = serde_json::json!(data.kind.code());
        detail["phase"] = serde_json::json!(data.diagnostic.phase);
        detail["safeCause"] = serde_json::json!(data.diagnostic.safe_cause);
        detail["sqliteCode"] = serde_json::json!(data.diagnostic.sqlite_code);
        detail["sqliteExtendedCode"] = serde_json::json!(data.diagnostic.sqlite_extended_code);
    }
    ToolOutcomeDiagnostic::try_from_value(detail)
        .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
        .unwrap_or(RunError::ToolOutcomeUnknown)
}

fn watermark(db: &Connection, child: Uuid, request: Uuid) -> Result<u64, CatalogError> {
    let record =
        super::repository::submissions::read(db, child, request).map_err(|error| match error {
            research_agent_core::submission::SubmissionError::Unavailable => {
                CatalogError::Unavailable
            }
            _ => CatalogError::Corrupt,
        })?;
    let Some(record) = record else { return Ok(0) };
    // Only immutable events for this request's fixed turn establish the phase.
    // A subsequent turn must never silently widen the cleanup authorization.
    let mut statement = db.prepare("select ordinal,payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type') in ('turnStarted','turnFinished') order by ordinal limit 3")
        .map_err(|_| CatalogError::Unavailable)?;
    let rows = statement
        .query_map(
            rusqlite::params![child.to_string(), record.turn_id.to_string()],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| CatalogError::Corrupt)?;
    let events = rows
        .map(|row| {
            let (ordinal, json) = row.map_err(|_| CatalogError::Corrupt)?;
            let event: AgentEvent =
                serde_json::from_str(&json).map_err(|_| CatalogError::Corrupt)?;
            if event.conversation_id().as_uuid() != &child
                || Some(event.seq()) != ordinal.checked_add(1)
            {
                return Err(CatalogError::Corrupt);
            }
            Ok((ordinal, event.payload().clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match events.as_slice() {
        [] => Ok(0),
        [(_, EventPayload::TurnStarted { .. })] => Err(CatalogError::Busy),
        [
            (start, EventPayload::TurnStarted { turn_id: first, .. }),
            (end, EventPayload::TurnFinished { turn_id: last, .. }),
        ] if first == &record.turn_id && last == first && end > start => end
            .checked_add(1)
            .filter(|value| *value <= 9_007_199_254_740_991)
            .ok_or(CatalogError::Corrupt),
        _ => Err(CatalogError::Corrupt),
    }
}

pub(crate) async fn archive(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    child: Uuid,
    request: Uuid,
) -> Result<(), ArchiveFailure> {
    archive_inner(store, catalog, child, request, false, Dispatch::Real).await
}

pub(crate) async fn archive_unsubmitted(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    child: Uuid,
    request: Uuid,
) -> Result<(), ArchiveFailure> {
    archive_inner(store, catalog, child, request, true, Dispatch::Real).await
}

#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) async fn preview_archive_unconfirmed(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    child: Uuid,
    request: Uuid,
) -> Result<(), ArchiveFailure> {
    archive_inner(
        store,
        catalog,
        child,
        request,
        false,
        Dispatch::PreviewUnconfirmed,
    )
    .await
}

enum Dispatch {
    Real,
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    PreviewUnconfirmed,
}

async fn archive_inner(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    child: Uuid,
    request: Uuid,
    require_absent: bool,
    dispatch: Dispatch,
) -> Result<(), ArchiveFailure> {
    let db = store.shared_agent_connection();
    let expected = tokio::task::spawn_blocking(move || {
        let db = db.lock().map_err(|_| CatalogError::Unavailable)?;
        watermark(&db, child, request)
    })
    .await
    .map_err(|_| CatalogError::Unavailable)
    .and_then(|value| value)
    .map_err(|error| ArchiveFailure {
        request_id: request,
        archive: None,
        reason: research_agent_core::conversation::CatalogMutationError::Domain(error).into(),
    })?;
    let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
    phase.extend_from_slice(request.as_bytes());
    let command = ArchiveConversation {
        operation_id: Uuid::new_v5(&child, &phase),
        id: ConversationId::from_uuid(child),
        archived: true,
        expected_watermark: Some(expected),
    };
    catalog
        .prepare_child_cleanup(
            crate::data_interface::ChildCleanupIntent {
                request_id: request,
                archive: command.clone(),
            },
            require_absent,
        )
        .await
        .map_err(|error| ArchiveFailure {
            request_id: request,
            archive: None,
            reason: error.into(),
        })?;
    let result = match dispatch {
        Dispatch::Real => catalog.set_archived(command.clone()).await.map(|_| ()),
        #[cfg(all(test, debug_assertions, feature = "development-data"))]
        Dispatch::PreviewUnconfirmed => Err(preview_unconfirmed(&command)),
    };
    result.map_err(|error| ArchiveFailure {
        request_id: request,
        archive: Some(ArchiveIntent {
            operation_id: command.operation_id,
            id: child,
            archived: true,
            expected_watermark: Some(expected),
        }),
        reason: error.into(),
    })
}

#[cfg(test)]
#[path = "child_archive/tests.rs"]
mod tests;
