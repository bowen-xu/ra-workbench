//! Workspace-owned delivery discovery. Listing grants neither execution nor writes.
use super::writing_workspace_commands::{Error, Scope};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[path = "writing_delivery_commands/repository.rs"]
mod repository;
#[path = "writing_delivery_commands/terminal.rs"]
mod terminal;
#[cfg(test)]
#[path = "writing_delivery_commands/tests.rs"]
mod tests;

#[cfg(target_os = "macos")]
pub(crate) fn latest_binding_for_scope(
    db: &Connection,
    scope: &Scope,
) -> Result<super::writing_workspace_commands::Confirmation, Error> {
    repository::latest_for_scope(db, scope)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub manuscript_id: Uuid,
    pub after: Option<u64>,
    pub limit: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    pub scope: Scope,
    pub turn_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub items: Vec<Entry>,
    pub next_cursor: Option<u64>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Acknowledge {
    pub scope: Scope,
    pub turn_id: Uuid,
}

#[tauri::command]
pub(crate) async fn agent_list_writing_file_deliveries(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Page, Error> {
    list(&store, request).await
}
pub(crate) async fn list(store: &AppStore, request: Request) -> Result<Page, Error> {
    if request.manuscript_id.is_nil()
        || request
            .after
            .is_some_and(|id| id == 0 || id > 9_007_199_254_740_991)
        || !(1..=64).contains(&request.limit)
    {
        return Err(Error::InvalidInput);
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        let page = repository::list(&tx, request)?;
        tx.commit().map_err(|_| Error::Unavailable)?;
        Ok(page)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_acknowledge_writing_file_delivery(
    store: State<'_, AppStore>,
    request: Acknowledge,
) -> Result<(), Error> {
    acknowledge(&store, request).await
}
pub(crate) async fn acknowledge(store: &AppStore, request: Acknowledge) -> Result<(), Error> {
    request.scope.validate()?;
    if request.turn_id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let check_store = store.clone();
    let check_request = request.clone();
    let settled = tokio::task::spawn_blocking(move || {
        let db = check_store.connection().map_err(|_| Error::Unavailable)?;
        repository::settlement_proven(&db, &check_request)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    if !settled {
        // Empty results have no proposal journal. Verify the actual revoked copy
        // through the existing native review/lease boundary, never trust the UI.
        #[cfg(target_os = "macos")]
        {
            let lookup = store.clone();
            let original = request.clone();
            let binding = tokio::task::spawn_blocking(move || {
                let db = lookup.connection().map_err(|_| Error::Unavailable)?;
                repository::latest_binding(&db, &original)
            })
            .await
            .map_err(|_| Error::Unavailable)??;
            let review = super::writing_review_commands::review_binding(store, binding).await?;
            if review.binding.turn_id != request.turn_id || !review.changes.is_empty() {
                return Err(Error::Conflict);
            }
        }
        #[cfg(not(target_os = "macos"))]
        return Err(Error::UnsupportedPlatform);
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Unavailable)?;
        repository::acknowledge(&tx, &request, !settled)?;
        tx.commit().map_err(|_| Error::Unavailable)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

/// Validate actual application facts while the turn is still running. The
/// transient tool response alone cannot prove that any file was delivered.
#[cfg(test)]
pub(crate) async fn verify_running(
    store: &AppStore,
    request: Acknowledge,
    delivery_id: Uuid,
) -> Result<RunningProof, Error> {
    verify_running_inner(store, request, delivery_id, None, None).await
}

pub(crate) async fn verify_running_with_call(
    store: &AppStore,
    request: Acknowledge,
    delivery_id: Uuid,
    provider_call_id: &str,
    binding: super::repository::execution_workspaces::Binding,
) -> Result<RunningProof, Error> {
    if provider_call_id.is_empty() {
        return Err(Error::InvalidInput);
    }
    verify_running_inner(
        store,
        request,
        delivery_id,
        Some(provider_call_id.to_owned()),
        Some(binding),
    )
    .await
}

async fn verify_running_inner(
    store: &AppStore,
    request: Acknowledge,
    delivery_id: Uuid,
    provider_call_id: Option<String>,
    binding: Option<super::repository::execution_workspaces::Binding>,
) -> Result<RunningProof, Error> {
    request.scope.validate()?;
    if request.turn_id.is_nil() || delivery_id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let service = super::writing_submission::WritingSubmissionService::from_app_store(store);
    let review = if let Some(binding) = binding {
        service.review_workspace_binding(binding).await
    } else {
        service
            .review_workspace(request.scope.receipt(), request.scope.context_snapshot_id)
            .await
    }
    .map_err(|_| Error::RecoveryRequired)?;
    if review.binding.turn_id != request.turn_id
        || review.binding.conversation_id != request.scope.conversation_id
        || review.binding.request_id != request.scope.request_id
        || review.binding.manuscript_id != request.scope.manuscript_id
        || review.binding.context_snapshot_id != request.scope.context_snapshot_id
    {
        return Err(Error::Conflict);
    }
    #[cfg(target_os = "macos")]
    if store.writing_workspace_source().is_none()
        && let Some(binding) = store.native_writing_context()
    {
        return binding
            .verify_running_proof(
                store.clone(),
                request,
                review,
                delivery_id,
                provider_call_id,
                service.worker_retention(),
            )
            .await;
    }
    let checked = store.clone();
    tokio::task::spawn_blocking(move || {
        use crate::ai::structured_resource_validation::Flow;
        proof_with_checkpoints(
            &checked,
            &request,
            &review,
            delivery_id,
            provider_call_id.as_deref(),
            |db, scope, path| {
                let read = checked
                    .writing_checkpoint_source()
                    .ok_or_else(|| Flow::original(Error::RecoveryRequired))?;
                read(db, scope.manuscript, path)
                    .map_err(|_| Flow::original(Error::RecoveryRequired))
            },
        )
        .map_err(Flow::<Error, std::convert::Infallible>::into_static)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(crate) fn proof_with_checkpoints<P>(
    store: &AppStore,
    request: &Acknowledge,
    review: &crate::agent::writing_submission::workspace::WorkspaceReview,
    delivery_id: Uuid,
    provider_call_id: Option<&str>,
    mut read_checkpoint: impl FnMut(
        &Connection,
        crate::agent::repository::proposal_applications::ApplicationScope,
        &str,
    ) -> Result<
        Option<crate::agent::writing_checkpoint_contracts::CheckpointRecord>,
        crate::ai::structured_resource_validation::Flow<Error, P>,
    >,
) -> Result<RunningProof, crate::ai::structured_resource_validation::Flow<Error, P>> {
    use crate::ai::structured_resource_validation::Flow;
    proof_transaction_with(store, |tx| {
        let proof =
            repository::running_proof_with(tx, request, review, delivery_id, |scope, path| {
                read_checkpoint(tx, scope, path)
            })?;
        if let Some(provider_call_id) = provider_call_id {
            repository::bind_running_call(tx, request, delivery_id, provider_call_id)
                .map_err(Flow::original)?;
        }
        Ok(proof)
    })
}

/// A failed commit or rollback can leave the shared SQL/log pair uncertain.
/// Do not allow subsequent admissions until it has been reopened and checked.
pub(super) fn proof_transaction<T>(
    store: &AppStore,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    proof_transaction_with(store, |tx| operation(tx).map_err(crate::ai::structured_resource_validation::Flow::original))
        .map_err(crate::ai::structured_resource_validation::Flow::<Error,std::convert::Infallible>::into_static)
}

pub(crate) fn proof_transaction_with<T, P>(
    store: &AppStore,
    operation: impl FnOnce(
        &rusqlite::Transaction<'_>,
    ) -> Result<T, crate::ai::structured_resource_validation::Flow<Error, P>>,
) -> Result<T, crate::ai::structured_resource_validation::Flow<Error, P>> {
    use crate::ai::structured_resource_validation::Flow;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Unavailable))?;
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Unavailable))?;
    tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
    match operation(&tx) {
        Ok(value) => match tx.commit() {
            Ok(()) if db.is_autocommit() => Ok(value),
            _ => {
                if !db.is_autocommit() {
                    let _ = db.execute_batch("rollback");
                }
                db.require_recovery();
                Err(Flow::original(Error::RecoveryRequired))
            }
        },
        Err(error) => match tx.rollback() {
            Ok(()) if db.is_autocommit() => Err(error),
            _ => {
                db.require_recovery();
                Err(Flow::original(Error::RecoveryRequired))
            }
        },
    }
}

pub(super) fn confirm_result_in(
    db: &Connection,
    record: &research_agent_core::submission::SubmissionRecord,
    provider_call_id: &str,
) -> Result<(), Error> {
    repository::confirm_result(db, record, provider_call_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunningProof {
    Applied,
    Rejected,
    Empty,
}
