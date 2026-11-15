//! User-facing execution consent boundary; never included in model tool catalogs.
use super::{
    service::AgentSessionService,
    writing_submission::{WritingSubmissionReceipt, WritingSubmissionService},
};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
    pub context_snapshot_id: Uuid,
}
impl Scope {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if [
            self.conversation_id,
            self.request_id,
            self.manuscript_id,
            self.context_snapshot_id,
        ]
        .iter()
        .any(Uuid::is_nil)
        {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
    pub(super) fn receipt(&self) -> WritingSubmissionReceipt {
        WritingSubmissionReceipt {
            conversation_id: self.conversation_id,
            request_id: self.request_id,
            manuscript_id: self.manuscript_id,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Buffer {
    pub path: String,
    pub expected_disk_sha256: String,
    pub session_id: Uuid,
    pub revision: Uuid,
    pub text: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub scope: Scope,
    pub buffers: Vec<Buffer>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareNextRequest {
    pub scope: Scope,
    pub generation: u32,
    pub predecessor_delivery_id: Uuid,
    pub lease_id: Uuid,
    pub buffers: Vec<Buffer>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextIdentity {
    pub scope: Scope,
    pub generation: u32,
    pub predecessor_delivery_id: Uuid,
    pub lease_id: Uuid,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Confirmation {
    pub scope: Scope,
    pub turn_id: Uuid,
    pub lease_id: Uuid,
    pub manifest_hash: String,
}
impl Confirmation {
    pub(super) fn binding(
        &self,
    ) -> Result<super::repository::execution_workspaces::Binding, Error> {
        self.scope.validate()?;
        if self.turn_id.is_nil()
            || self.lease_id.is_nil()
            || self.manifest_hash.len() != 64
            || !self
                .manifest_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::InvalidInput);
        }
        Ok(super::repository::execution_workspaces::Binding {
            conversation_id: self.scope.conversation_id,
            request_id: self.scope.request_id,
            manuscript_id: self.scope.manuscript_id,
            context_snapshot_id: self.scope.context_snapshot_id,
            turn_id: self.turn_id,
            lease_id: self.lease_id,
            manifest_hash: self.manifest_hash.clone(),
        })
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    path: String,
    byte_size: u64,
    has_editor_buffer: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    schema_version: u8,
    conversation_id: Uuid,
    request_id: Uuid,
    manuscript_id: Uuid,
    context_snapshot_id: Uuid,
    turn_id: Uuid,
    lease_id: Uuid,
    manifest_hash: String,
    policy: &'static str,
    approved: bool,
    files: Vec<FilePreview>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    Busy,
    Conflict,
    NotFound,
    Denied,
    Unavailable,
    RecoveryRequired,
    UnsupportedPlatform,
    #[serde(untagged)]
    Source(Box<super::writing_commands::PrepareCommandError>),
}

#[cfg(target_os = "macos")]
fn preparation_error(cause: super::writing_submission::workspace::Error, scope: &Scope) -> Error {
    source_error(
        cause,
        scope.conversation_id,
        scope.manuscript_id,
        scope.request_id,
        error,
    )
}

#[cfg(target_os = "macos")]
pub(super) fn source_error(
    cause: super::writing_submission::workspace::Error,
    conversation_id: Uuid,
    manuscript_id: Uuid,
    operation_id: Uuid,
    fallback: fn(super::writing_submission::workspace::Error) -> Error,
) -> Error {
    use super::writing_submission::{WritingSubmissionError, workspace};
    match cause {
        workspace::Error::Submission(WritingSubmissionError::Native(cause)) => Error::Source(
            Box::new(super::writing_commands::PrepareCommandError::Native {
                cause,
                conversation_id,
                manuscript_id,
                operation_id,
            }),
        ),
        cause => fallback(cause),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn error(error: super::writing_submission::workspace::Error) -> Error {
    use super::{
        repository::execution_workspaces,
        writing_submission::{WritingSubmissionError, workspace},
    };
    match error {
        workspace::Error::NotFound => Error::NotFound,
        workspace::Error::Binding(execution_workspaces::Error::Conflict) => Error::Conflict,
        workspace::Error::Binding(execution_workspaces::Error::Denied)
        | workspace::Error::Submission(WritingSubmissionError::Unbound) => Error::Denied,
        workspace::Error::RecoveryRequired => Error::RecoveryRequired,
        _ => Error::Unavailable,
    }
}
#[cfg(target_os = "macos")]
async fn preview(
    store: &AppStore,
    prepared: super::writing_submission::workspace::PreparedWorkspace,
) -> Result<Preview, Error> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        let binding = prepared.binding;
        let manifest = prepared.lease.manifest();
        let approved =
            match super::repository::execution_workspaces::authorize(&db, &binding, manifest) {
                Ok(()) => true,
                Err(super::repository::execution_workspaces::Error::Denied) => false,
                Err(_) => return Err(Error::Conflict),
            };
        Ok(Preview {
            schema_version: 1,
            conversation_id: binding.conversation_id,
            request_id: binding.request_id,
            manuscript_id: binding.manuscript_id,
            context_snapshot_id: binding.context_snapshot_id,
            turn_id: binding.turn_id,
            lease_id: binding.lease_id,
            manifest_hash: binding.manifest_hash,
            policy: "offline_workspace_v1",
            approved,
            files: manifest
                .files
                .iter()
                .map(|f| FilePreview {
                    path: f.file.path.clone(),
                    byte_size: f.file.version.byte_size,
                    has_editor_buffer: f.source.editor.is_some(),
                })
                .collect(),
        })
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
#[cfg(target_os = "macos")]
pub(crate) async fn prepare(store: &AppStore, request: PrepareRequest) -> Result<Preview, Error> {
    request.scope.validate()?;
    let service = WritingSubmissionService::from_app_store(store);
    let prepared = service
        .prepare_workspace(
            request.scope.receipt(),
            request.scope.context_snapshot_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: request.scope.manuscript_id,
                buffers: request
                    .buffers
                    .into_iter()
                    .map(|b| crate::agent::writing_workspace::EditorBuffer {
                        path: b.path,
                        expected_disk_sha256: b.expected_disk_sha256,
                        text: b.text,
                        editor: research_agent_core::workspace::EditorVersion {
                            session_id: b.session_id,
                            revision: b.revision,
                        },
                    })
                    .collect(),
            },
        )
        .await
        .map_err(|cause| preparation_error(cause, &request.scope))?;
    // Editing is enabled by default for writing submissions. The copy remains
    // isolated from the live manuscript; the user reviews its diff before it
    // can be published back to the source.
    let approved = service.approve_workspace(prepared).await.map_err(error)?;
    preview(store, approved).await
}

/// Prepare and approve only a successor with a confirmed predecessor and a
/// fresh, complete editor-buffer snapshot. The supplied lease identifies the
/// original attempt and must be retained if the response is lost.
#[cfg(target_os = "macos")]
pub(crate) async fn prepare_next(
    store: &AppStore,
    request: PrepareNextRequest,
) -> Result<Confirmation, Error> {
    request.scope.validate()?;
    if request.generation == 0
        || request.predecessor_delivery_id.is_nil()
        || request.lease_id.is_nil()
    {
        return Err(Error::InvalidInput);
    }
    let service = WritingSubmissionService::from_app_store(store);
    let prepared = service
        .prepare_next_workspace(
            request.scope.receipt(),
            request.scope.context_snapshot_id,
            request.generation,
            request.predecessor_delivery_id,
            request.lease_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: request.scope.manuscript_id,
                buffers: request
                    .buffers
                    .into_iter()
                    .map(|b| crate::agent::writing_workspace::EditorBuffer {
                        path: b.path,
                        expected_disk_sha256: b.expected_disk_sha256,
                        text: b.text,
                        editor: research_agent_core::workspace::EditorVersion {
                            session_id: b.session_id,
                            revision: b.revision,
                        },
                    })
                    .collect(),
            },
        )
        .await
        .map_err(|cause| preparation_error(cause, &request.scope))?;
    let approved = service
        .approve_next_workspace(
            prepared,
            request.generation,
            request.predecessor_delivery_id,
        )
        .await
        .map_err(|_| Error::RecoveryRequired)?;
    let binding = approved.binding;
    Ok(Confirmation {
        scope: request.scope,
        turn_id: binding.turn_id,
        lease_id: binding.lease_id,
        manifest_hash: binding.manifest_hash,
    })
}

/// Reconcile only the original attempt; never recreate a missing copy.
#[cfg(target_os = "macos")]
pub(crate) async fn recover_next(
    store: &AppStore,
    request: NextIdentity,
) -> Result<Confirmation, Error> {
    use rusqlite::OptionalExtension;
    request.scope.validate()?;
    if request.generation == 0
        || request.predecessor_delivery_id.is_nil()
        || request.lease_id.is_nil()
    {
        return Err(Error::InvalidInput);
    }
    let lookup = store.clone();
    let scope = request.scope.clone();
    let generation = request.generation;
    let predecessor = request.predecessor_delivery_id;
    let lease = request.lease_id;
    let state = tokio::task::spawn_blocking(move || {
        let db = lookup.connection().map_err(|_| Error::Unavailable)?;
        db.query_row("select state from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and manuscript_id=?3 and context_snapshot_id=?4 and generation=?5 and predecessor_delivery_id=?6 and lease_id=?7", rusqlite::params![scope.conversation_id.to_string(),scope.request_id.to_string(),scope.manuscript_id.to_string(),scope.context_snapshot_id.to_string(),generation,predecessor.to_string(),lease.to_string()], |r|r.get::<_,String>(0)).optional().map_err(|_| Error::Unavailable)?.ok_or(Error::NotFound)
    }).await.map_err(|_| Error::Unavailable)??;
    let service = WritingSubmissionService::from_app_store(store);
    let receipt = request.scope.receipt();
    let context = request.scope.context_snapshot_id;
    let approved = match state.as_str() {
        "preparing" => {
            let prepared = service
                .recover_next_workspace(receipt, context, generation, predecessor, lease)
                .await
                .map_err(error)?;
            service
                .approve_next_workspace(prepared, generation, predecessor)
                .await
                .map_err(|_| Error::RecoveryRequired)?
        }
        "prepared" => {
            let prepared = service
                .restore_prepared_next_workspace(receipt, context, generation, predecessor, lease)
                .await
                .map_err(error)?;
            service
                .approve_next_workspace(prepared, generation, predecessor)
                .await
                .map_err(|_| Error::RecoveryRequired)?
        }
        "approved" => service
            .restore_next_workspace(receipt, context, generation, predecessor, lease)
            .await
            .map_err(error)?,
        "revoked" => return Err(Error::Denied),
        _ => return Err(Error::Conflict),
    };
    let binding = approved.binding;
    Ok(Confirmation {
        scope: request.scope,
        turn_id: binding.turn_id,
        lease_id: binding.lease_id,
        manifest_hash: binding.manifest_hash,
    })
}

#[tauri::command]
pub(crate) async fn agent_recover_next_writing_workspace(
    store: State<'_, AppStore>,
    request: NextIdentity,
) -> Result<Confirmation, Error> {
    #[cfg(target_os = "macos")]
    {
        recover_next(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[tauri::command]
pub(crate) async fn agent_prepare_next_writing_workspace(
    store: State<'_, AppStore>,
    request: PrepareNextRequest,
) -> Result<Confirmation, Error> {
    #[cfg(target_os = "macos")]
    {
        prepare_next(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[cfg(target_os = "macos")]
pub(crate) async fn restore(store: &AppStore, scope: Scope) -> Result<Preview, Error> {
    scope.validate()?;
    let service = WritingSubmissionService::from_app_store(store);
    let prepared = service
        .restore_workspace(scope.receipt(), scope.context_snapshot_id)
        .await
        .map_err(error)?;
    let approved = service.approve_workspace(prepared).await.map_err(error)?;
    preview(store, approved).await
}
#[cfg(target_os = "macos")]
pub(crate) async fn approve(store: &AppStore, request: Confirmation) -> Result<Preview, Error> {
    let expected = request.binding()?;
    let service = WritingSubmissionService::from_app_store(store);
    let prepared = service
        .restore_workspace(request.scope.receipt(), request.scope.context_snapshot_id)
        .await
        .map_err(error)?;
    if prepared.binding != expected {
        return Err(Error::Conflict);
    }
    preview(
        store,
        service.approve_workspace(prepared).await.map_err(error)?,
    )
    .await
}
#[cfg(target_os = "macos")]
pub(crate) async fn revoke(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: Confirmation,
) -> Result<(), Error> {
    WritingSubmissionService::from_app_store(store)
        .revoke_workspace(sessions, request.binding()?)
        .await
        .map_err(error)
}

#[tauri::command]
pub(crate) async fn agent_prepare_writing_workspace(
    store: State<'_, AppStore>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    #[cfg(target_os = "macos")]
    {
        prepare(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_restore_writing_workspace(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Preview, Error> {
    #[cfg(target_os = "macos")]
    {
        restore(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_approve_writing_workspace(
    store: State<'_, AppStore>,
    request: Confirmation,
) -> Result<Preview, Error> {
    #[cfg(target_os = "macos")]
    {
        approve(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_revoke_writing_workspace(
    store: State<'_, AppStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Confirmation,
) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        revoke(&store, sessions.inner().clone(), request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, sessions, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "writing_workspace_commands/tests.rs"]
mod tests;
