//! Typed proof and read-only inspection for independent-context preparation.
use super::{PrepareRequest, Preview};
use crate::{
    agent::{
        catalog_commands::CatalogMutationProofDto,
        model::{AgentHost, AgentModelResolver},
    },
    data_interface::{
        ContextPrepareIntent, ContextPrepareSource, ContextPrepareStatus, SqlImmediateDiagnostic,
        SqlImmediateFailure, SqlImmediateReceipt,
    },
    storage::AppStore,
};
use research_agent_core::conversation::{CatalogMutationProof, ConversationId};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

const FAMILY: &str = "agent.context.submission.prepare";
#[path = "prepare_status/source_error.rs"]
mod source_error;
#[derive(Debug, Serialize)]
#[serde(tag = "code", content = "reason", rename_all = "camelCase")]
pub(crate) enum Error {
    Storage(SqlImmediateFailure),
    ModelUnavailable,
    #[serde(untagged)]
    Source(Box<source_error::ContextSourceFailure>),
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Status {
    Committed {
        proof: CatalogMutationProofDto,
        result: Box<Preview>,
    },
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    schema_version: u8,
    intent: PrepareRequest,
    expected_epoch: Option<Uuid>,
    status: Status,
}
fn committed(
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
    preview: Preview,
    receipt: SqlImmediateReceipt,
) -> Response {
    let proof = CatalogMutationProofDto::new(
        CatalogMutationProof {
            operation_id: receipt.operation_id,
            conversation_id: ConversationId::from_uuid(request.conversation_id),
            epoch: receipt.epoch,
            sql_fence: receipt.sql_fence.position,
            replayed: receipt.replayed,
        },
        FAMILY,
    );
    Response {
        schema_version: 1,
        intent: request,
        expected_epoch,
        status: Status::Committed {
            proof,
            result: Box::new(preview),
        },
    }
}
fn worker_failure(operation_id: Uuid, writing: bool) -> Error {
    let detail = Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase: if writing {
            "context-prepare-worker"
        } else {
            "context-prepare-status-worker"
        },
        sqlite_code: None,
        sqlite_extended_code: None,
        safe_cause: if writing {
            "context_prepare_outcome_unknown"
        } else {
            "context_prepare_status_unavailable"
        },
    });
    Error::Storage(if writing {
        SqlImmediateFailure::Unknown(detail)
    } else {
        SqlImmediateFailure::Unavailable(detail)
    })
}
#[tauri::command]
pub(crate) async fn agent_read_context_submission_prepare_status(
    store: State<'_, AppStore>,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<Response, Error> {
    read(store.inner(), request, expected_epoch).await
}
pub(crate) async fn read(
    store: &AppStore,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<Response, Error> {
    let operation = request.request_id;
    let database = store.shared_agent_connection();
    let command = ContextPrepareIntent {
        request: request.clone(),
        expected_epoch,
    };
    let status = tokio::task::spawn_blocking(move || database.context_prepare_status(&command))
        .await
        .map_err(|_| worker_failure(operation, false))?
        .map_err(Error::Storage)?;
    Ok(match status {
        ContextPrepareStatus::Committed { preview, receipt } => {
            committed(request, expected_epoch, *preview, *receipt)
        }
        ContextPrepareStatus::Unconfirmed(detail) => Response {
            schema_version: 1,
            intent: request,
            expected_epoch,
            status: Status::Unconfirmed { detail },
        },
    })
}
#[tauri::command]
pub(crate) async fn agent_prepare_context_submission_proven(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<Response, Error> {
    prepare(store.inner(), models.inner(), request, expected_epoch).await
}
pub(crate) async fn prepare(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<Response, Error> {
    let operation = request.request_id;
    let database = store.shared_agent_connection();
    let db = database.clone();
    let command = ContextPrepareIntent {
        request: request.clone(),
        expected_epoch,
    };
    if let Some((preview, receipt)) =
        tokio::task::spawn_blocking(move || db.replay_context_prepare(&command))
            .await
            .map_err(|_| worker_failure(operation, false))?
            .map_err(Error::Storage)?
    {
        return Ok(committed(request, expected_epoch, preview, receipt));
    }
    let model = models
        .prepare(AgentHost::General, request.model.clone().map(Into::into))
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let selection = (
        model.identity().clone(),
        model.reasoning_effort().map(str::to_owned),
        model.service_tier().map(str::to_owned),
    );
    drop(model);
    let command = ContextPrepareIntent {
        request: request.clone(),
        expected_epoch,
    };
    let source = ContextPrepareSource {
        store: store.clone(),
        model: selection,
    };
    let (preview, receipt) = if let Some(binding) = store.native_writing_context().filter(|_| {
        command.request.resources.iter().any(|resource| {
            matches!(
                resource,
                super::sources::Resource::Manuscript { .. }
                    | super::sources::Resource::ManuscriptFile { .. }
            )
        })
    }) {
        binding
            .prepare_independent(database, command, source)
            .await?
    } else {
        tokio::task::spawn_blocking(move || database.prepare_context_owned(&command, source))
            .await
            .map_err(|_| worker_failure(operation, true))?
            .map_err(Error::Storage)?
    };
    Ok(committed(request, expected_epoch, preview, receipt))
}

#[cfg(all(test, target_os = "macos"))]
#[path = "prepare_status/runtime_tests.rs"]
mod runtime_tests;
