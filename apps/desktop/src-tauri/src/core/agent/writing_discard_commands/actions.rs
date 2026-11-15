use super::*;
use crate::agent::{service::AgentSessionService, writing_review_commands::Review};
use std::sync::Arc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    schema_version: u8,
    confirmation: DiscardConfirmation,
    review: Review,
}

#[cfg(target_os = "macos")]
fn error(error: crate::agent::writing_submission::workspace::Error) -> Error {
    use crate::agent::{workspace::LeaseError, writing_submission::workspace};
    match error {
        workspace::Error::Lease(LeaseError::Busy) => Error::Busy,
        workspace::Error::Lease(LeaseError::Conflict) => Error::Conflict,
        other => crate::agent::writing_workspace_commands::error(other),
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn preview(store: AppStore, scope: Scope) -> Result<Preview, Error> {
    use crate::agent::{
        repository::execution_workspaces::discard, writing_submission::WritingSubmissionService,
    };
    use sha2::{Digest, Sha256};
    scope.validate()?;
    if lookup(store.clone(), scope.clone()).await?.is_some() {
        return Err(Error::RecoveryRequired);
    }
    let reviewed = WritingSubmissionService::from_app_store(&store)
        .review_workspace(scope.receipt(), scope.context_snapshot_id)
        .await
        .map_err(error)?;
    let inventory_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&reviewed.changes).map_err(|_| Error::Unavailable)?)
    );
    let confirmation: DiscardConfirmation = discard::Request {
        operation_id: Uuid::new_v4(),
        binding: reviewed.binding,
        environment_id: reviewed.baseline.identity.environment_id,
        inventory_hash,
    }
    .into();
    Ok(Preview {
        schema_version: 1,
        review: Review {
            schema_version: 1,
            binding: confirmation.binding.clone(),
            changes: reviewed.changes.into_iter().map(Into::into).collect(),
        },
        confirmation,
    })
}

#[cfg(target_os = "macos")]
pub(crate) async fn execute(
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    request: DiscardConfirmation,
) -> Result<Recovery, Error> {
    use crate::agent::writing_submission::WritingSubmissionService;
    let native = request.native()?;
    // UI authority is checked even for completed receipts. The native coordinator
    // separately supports exact reconciliation after host deletion.
    if let Some(saved) = lookup(store.clone(), request.binding.scope.clone()).await?
        && saved.confirmation.native()? != native
    {
        return Err(Error::Conflict);
    }
    sessions
        .discard_workspace(WritingSubmissionService::from_app_store(&store), native)
        .await
        .map_err(error)?;
    Ok(Recovery {
        schema_version: 1,
        confirmation: request,
        state: RecoveryState::Complete,
    })
}

#[tauri::command]
pub(crate) async fn agent_preview_writing_workspace_discard(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Preview, Error> {
    #[cfg(target_os = "macos")]
    {
        preview(store.inner().clone(), request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[tauri::command]
pub(crate) async fn agent_discard_writing_workspace(
    store: State<'_, AppStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: DiscardConfirmation,
) -> Result<Recovery, Error> {
    #[cfg(target_os = "macos")]
    {
        execute(store.inner().clone(), sessions.inner().clone(), request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, sessions, request);
        Err(Error::UnsupportedPlatform)
    }
}
