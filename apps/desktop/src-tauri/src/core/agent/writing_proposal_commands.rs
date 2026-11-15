//! Frozen proposal boundary. Publication preserves a review, never applies it.
use super::{
    writing_review_commands::{FileReview, Review, Version},
    writing_workspace_commands::{Confirmation, Error, Scope},
};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedFile {
    pub path: String,
    pub before: Option<Version>,
    pub after: Option<Version>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishRequest {
    pub binding: Confirmation,
    pub changes: Vec<ExpectedFile>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub proposal_id: Uuid,
}
impl ProposalRequest {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if [self.conversation_id, self.manuscript_id, self.proposal_id]
            .iter()
            .any(Uuid::is_nil)
        {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalFileRequest {
    pub proposal: ProposalRequest,
    pub path: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalReview {
    schema_version: u8,
    proposal_id: Uuid,
    status: super::repository::change_sets::ChangeSetStatus,
    review: Review,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalFileReview {
    schema_version: u8,
    proposal_id: Uuid,
    file: FileReview,
}
fn confirmation(binding: super::repository::execution_workspaces::Binding) -> Confirmation {
    Confirmation {
        scope: Scope {
            conversation_id: binding.conversation_id,
            request_id: binding.request_id,
            manuscript_id: binding.manuscript_id,
            context_snapshot_id: binding.context_snapshot_id,
        },
        turn_id: binding.turn_id,
        lease_id: binding.lease_id,
        manifest_hash: binding.manifest_hash,
    }
}
#[cfg(target_os = "macos")]
pub(crate) async fn read(
    store: &AppStore,
    request: ProposalRequest,
) -> Result<ProposalReview, Error> {
    request.validate()?;
    let frozen = super::writing_submission::WritingSubmissionService::from_app_store(store)
        .review_proposal(
            request.conversation_id,
            request.manuscript_id,
            request.proposal_id,
        )
        .await
        .map_err(super::writing_workspace_commands::error)?;
    Ok(project(frozen))
}
fn project(frozen: super::repository::change_sets::StoredChangeSet) -> ProposalReview {
    ProposalReview {
        schema_version: 1,
        proposal_id: frozen.id,
        status: frozen.status,
        review: Review {
            schema_version: 1,
            binding: confirmation(frozen.binding),
            changes: frozen.changes.into_iter().map(Into::into).collect(),
        },
    }
}
#[cfg(target_os = "macos")]
pub(crate) async fn reject(
    store: &AppStore,
    request: ProposalRequest,
) -> Result<ProposalReview, Error> {
    request.validate()?;
    super::writing_submission::WritingSubmissionService::from_app_store(store)
        .reject_proposal(
            request.conversation_id,
            request.manuscript_id,
            request.proposal_id,
        )
        .await
        .map(project)
        .map_err(super::writing_workspace_commands::error)
}

#[tauri::command]
pub(crate) async fn agent_reject_writing_proposal(
    store: State<'_, AppStore>,
    request: ProposalRequest,
) -> Result<ProposalReview, Error> {
    #[cfg(target_os = "macos")]
    {
        reject(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[cfg(target_os = "macos")]
pub(crate) async fn publish(
    store: &AppStore,
    request: PublishRequest,
) -> Result<ProposalReview, Error> {
    let binding = request.binding.binding()?;
    if request.changes.is_empty() || request.changes.len() > 20_000 {
        return Err(Error::InvalidInput);
    }
    let mut expected = Vec::with_capacity(request.changes.len());
    for file in request.changes {
        research_agent_core::workspace::validate_path(&file.path)
            .map_err(|_| Error::InvalidInput)?;
        expected.push((
            file.path,
            file.before.map(Version::native).transpose()?,
            file.after.map(Version::native).transpose()?,
        ));
    }
    let service = super::writing_submission::WritingSubmissionService::from_app_store(store);
    let live = service
        .review_workspace_binding(binding.clone())
        .await
        .map_err(super::writing_workspace_commands::error)?;
    if live.binding != binding
        || live.changes.len() != expected.len()
        || live
            .changes
            .iter()
            .zip(expected)
            .any(|(actual, (path, before, after))| {
                actual.path != path || actual.before != before || actual.after != after
            })
    {
        return Err(Error::Conflict);
    }
    let id = service
        .publish_workspace_changes(binding.clone(), live.changes)
        .await
        .map_err(super::writing_workspace_commands::error)?;
    read(
        store,
        ProposalRequest {
            conversation_id: binding.conversation_id,
            manuscript_id: binding.manuscript_id,
            proposal_id: id,
        },
    )
    .await
}
#[cfg(target_os = "macos")]
pub(crate) async fn read_file(
    store: &AppStore,
    request: ProposalFileRequest,
) -> Result<ProposalFileReview, Error> {
    request.proposal.validate()?;
    research_agent_core::workspace::validate_path(&request.path)
        .map_err(|_| Error::InvalidInput)?;
    let service = super::writing_submission::WritingSubmissionService::from_app_store(store);
    let scope = request.proposal;
    let frozen = service
        .review_proposal(
            scope.conversation_id,
            scope.manuscript_id,
            scope.proposal_id,
        )
        .await
        .map_err(super::writing_workspace_commands::error)?;
    let content = service
        .review_proposal_file(
            scope.conversation_id,
            scope.manuscript_id,
            scope.proposal_id,
            request.path,
        )
        .await
        .map_err(super::writing_workspace_commands::error)?;
    Ok(ProposalFileReview {
        schema_version: 1,
        proposal_id: scope.proposal_id,
        file: FileReview {
            schema_version: 1,
            binding: confirmation(frozen.binding),
            change: content.change.into(),
            before: content.before.map(Into::into),
            after: content.after.map(Into::into),
        },
    })
}
#[tauri::command]
pub(crate) async fn agent_publish_writing_proposal(
    store: State<'_, AppStore>,
    request: PublishRequest,
) -> Result<ProposalReview, Error> {
    #[cfg(target_os = "macos")]
    {
        publish(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_review_writing_proposal(
    store: State<'_, AppStore>,
    request: ProposalRequest,
) -> Result<ProposalReview, Error> {
    #[cfg(target_os = "macos")]
    {
        read(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_review_writing_proposal_file(
    store: State<'_, AppStore>,
    request: ProposalFileRequest,
) -> Result<ProposalFileReview, Error> {
    #[cfg(target_os = "macos")]
    {
        read_file(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn latest(
    store: &AppStore,
    scope: Scope,
) -> Result<Option<ProposalReview>, Error> {
    use rusqlite::{OptionalExtension, params};
    scope.validate()?;
    let owned = store.clone();
    let query = scope.clone();
    let id = tokio::task::spawn_blocking(move || {
        let db = owned.connection().map_err(|_| Error::Unavailable)?;
        let has_workspace: bool = db.query_row("select exists(select 1 from agent_execution_workspaces where conversation_id=?1 and request_id=?2 and manuscript_id=?3 and context_snapshot_id=?4)",
            params![query.conversation_id.to_string(),query.request_id.to_string(),query.manuscript_id.to_string(),query.context_snapshot_id.to_string()],|r|r.get(0)).map_err(|_|Error::Unavailable)?;
        if !has_workspace { return Ok(None); }
        let binding = super::writing_delivery_commands::latest_binding_for_scope(&db, &query)?;
        let value: Option<String> = db.query_row("select id from agent_change_sets where conversation_id=?1 and manuscript_id=?2 and request_id=?3 and context_snapshot_id=?4 and turn_id=?5 and lease_id=?6 and manifest_hash=?7 order by created_at desc,id desc limit 1",params![query.conversation_id.to_string(),query.manuscript_id.to_string(),query.request_id.to_string(),query.context_snapshot_id.to_string(),binding.turn_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash],|r|r.get(0)).optional().map_err(|_|Error::Unavailable)?;
        value.map(|id| Uuid::parse_str(&id).map_err(|_|Error::Conflict)).transpose()
    }).await.map_err(|_|Error::Unavailable)??;
    match id {
        Some(proposal_id) => read(
            store,
            ProposalRequest {
                conversation_id: scope.conversation_id,
                manuscript_id: scope.manuscript_id,
                proposal_id,
            },
        )
        .await
        .map(Some),
        None => Ok(None),
    }
}
#[tauri::command]
pub(crate) async fn agent_latest_writing_proposal(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Option<ProposalReview>, Error> {
    #[cfg(target_os = "macos")]
    {
        latest(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
