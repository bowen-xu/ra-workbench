//! Read-only change review. Never registers tools or grants execution/write access.
use super::writing_workspace_commands::{Confirmation, Error, Scope};
use crate::storage::AppStore;
use research_agent_core::workspace::{FileChange, FileVersion};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Version {
    pub sha256: String,
    pub byte_size: u64,
    pub executable: bool,
}
impl Version {
    pub(super) fn native(self) -> Result<FileVersion, Error> {
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.byte_size > research_agent_core::workspace::MAX_FILE_BYTES
        {
            return Err(Error::InvalidInput);
        }
        Ok(FileVersion {
            sha256: self.sha256,
            byte_size: self.byte_size,
            executable: self.executable,
        })
    }
}
impl From<FileVersion> for Version {
    fn from(value: FileVersion) -> Self {
        Self {
            sha256: value.sha256,
            byte_size: value.byte_size,
            executable: value.executable,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRequest {
    pub binding: Confirmation,
    pub path: String,
    pub before: Option<Version>,
    pub after: Option<Version>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    path: String,
    kind: research_agent_core::workspace::FileChangeKind,
    before: Option<Version>,
    after: Option<Version>,
    has_editor_buffer: bool,
}
impl From<FileChange> for Change {
    fn from(value: FileChange) -> Self {
        Self {
            path: value.path,
            kind: value.kind,
            before: value.before.map(Into::into),
            after: value.after.map(Into::into),
            has_editor_buffer: value.source.is_some_and(|source| source.editor.is_some()),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub(super) schema_version: u8,
    pub(super) binding: Confirmation,
    pub(super) changes: Vec<Change>,
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "text", rename_all = "camelCase")]
pub enum Content {
    Text(String),
    Binary,
    TooLarge,
}
#[cfg(target_os = "macos")]
impl From<super::workspace::ReviewContent> for Content {
    fn from(value: super::workspace::ReviewContent) -> Self {
        match value {
            super::workspace::ReviewContent::Text(text) => Self::Text(text),
            super::workspace::ReviewContent::Binary => Self::Binary,
            super::workspace::ReviewContent::TooLarge => Self::TooLarge,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReview {
    pub(super) schema_version: u8,
    pub(super) binding: Confirmation,
    pub(super) change: Change,
    pub(super) before: Option<Content>,
    pub(super) after: Option<Content>,
}
#[cfg(target_os = "macos")]
pub(crate) async fn review(store: &AppStore, scope: Scope) -> Result<Review, Error> {
    scope.validate()?;
    let owned = store.clone();
    let binding = tokio::task::spawn_blocking(move || {
        let db = owned.connection().map_err(|_| Error::Unavailable)?;
        super::writing_delivery_commands::latest_binding_for_scope(&db, &scope)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    review_binding(store, binding).await
}

#[cfg(target_os = "macos")]
pub(crate) async fn review_binding(
    store: &AppStore,
    request: Confirmation,
) -> Result<Review, Error> {
    let binding = request.binding()?;
    let value = super::writing_submission::WritingSubmissionService::from_app_store(store)
        .review_workspace_binding(binding)
        .await
        .map_err(super::writing_workspace_commands::error)?;
    Ok(Review {
        schema_version: 1,
        binding: request,
        changes: value.changes.into_iter().map(Into::into).collect(),
    })
}

#[tauri::command]
pub(crate) async fn agent_review_writing_workspace_binding(
    store: State<'_, AppStore>,
    request: Confirmation,
) -> Result<Review, Error> {
    #[cfg(target_os = "macos")]
    {
        review_binding(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[cfg(target_os = "macos")]
pub(crate) async fn review_file(
    store: &AppStore,
    request: FileRequest,
) -> Result<FileReview, Error> {
    let binding = request.binding.binding()?;
    research_agent_core::workspace::validate_path(&request.path)
        .map_err(|_| Error::InvalidInput)?;
    let before = request.before.map(Version::native).transpose()?;
    let after = request.after.map(Version::native).transpose()?;
    if before.is_none() && after.is_none() {
        return Err(Error::InvalidInput);
    }
    let content = super::writing_submission::WritingSubmissionService::from_app_store(store)
        .review_workspace_change(binding, request.path, before, after)
        .await
        .map_err(super::writing_workspace_commands::error)?;
    Ok(FileReview {
        schema_version: 1,
        binding: request.binding,
        change: content.change.into(),
        before: content.before.map(Into::into),
        after: content.after.map(Into::into),
    })
}
#[tauri::command]
pub(crate) async fn agent_review_writing_workspace(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Review, Error> {
    #[cfg(target_os = "macos")]
    {
        review(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
#[tauri::command]
pub(crate) async fn agent_review_writing_workspace_file(
    store: State<'_, AppStore>,
    request: FileRequest,
) -> Result<FileReview, Error> {
    #[cfg(target_os = "macos")]
    {
        review_file(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryRequest {
    pub conversation_id: uuid::Uuid,
    pub manuscript_id: uuid::Uuid,
    pub cursor: Option<uuid::Uuid>,
    pub limit: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    scope: Scope,
    turn_id: uuid::Uuid,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_proposal_id: Option<uuid::Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    schema_version: u8,
    entries: Vec<HistoryEntry>,
    next_cursor: Option<uuid::Uuid>,
}
#[cfg(target_os = "macos")]
pub(crate) async fn history(
    store: &AppStore,
    request: HistoryRequest,
) -> Result<HistoryPage, Error> {
    if request.conversation_id.is_nil()
        || request.manuscript_id.is_nil()
        || request.cursor.is_some_and(|id| id.is_nil())
        || !(1..=100).contains(&request.limit)
    {
        return Err(Error::InvalidInput);
    }
    let page = super::writing_submission::WritingSubmissionService::from_app_store(store)
        .list_workspace_reviews(
            request.conversation_id,
            request.manuscript_id,
            request.cursor,
            request.limit,
        )
        .await
        .map_err(super::writing_workspace_commands::error)?;
    Ok(HistoryPage {
        schema_version: 1,
        entries: page
            .entries
            .into_iter()
            .map(|entry| HistoryEntry {
                scope: Scope {
                    conversation_id: request.conversation_id,
                    manuscript_id: request.manuscript_id,
                    request_id: entry.request_id,
                    context_snapshot_id: entry.context_snapshot_id,
                },
                turn_id: entry.turn_id,
                created_at: entry.created_at,
                published_proposal_id: entry.published_proposal_id,
            })
            .collect(),
        next_cursor: page.next_cursor,
    })
}
#[tauri::command]
pub(crate) async fn agent_list_writing_workspace_reviews(
    store: State<'_, AppStore>,
    request: HistoryRequest,
) -> Result<HistoryPage, Error> {
    #[cfg(target_os = "macos")]
    {
        history(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
