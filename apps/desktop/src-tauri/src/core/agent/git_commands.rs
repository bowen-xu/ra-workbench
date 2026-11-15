//! User-facing manuscript Git version commands.
//!
//! This is deliberately a thin native boundary over `GitWorkspaceService`.
//! It does not expose repository paths and never stages files on behalf of the
//! caller; the user must have explicitly staged the snapshot first.

use super::git::{GitServiceError, GitWorkspaceService};
use crate::storage::AppStore;
use research_agent_core::git::GitCommitSummary;
use research_agent_core::workspace::WorkspaceFile;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::Path;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateVersionRequest {
    pub manuscript_id: Uuid,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptVersion {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub commit: GitCommitSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListVersionsRequest {
    pub manuscript_id: Uuid,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitializeRequest {
    pub manuscript_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptVersionHistory {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub commits: Vec<GitCommitSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepositoryView {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub root: String,
    pub branch: Option<String>,
    pub head: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptGitBranches {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub current: Option<String>,
    pub branches: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchRequest {
    pub manuscript_id: Uuid,
    pub branch: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffVersionsRequest {
    pub manuscript_id: Uuid,
    pub from_revision: String,
    pub to_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreVersionRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub revision: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptGitRestoreProposal {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub revision: String,
    pub proposal_id: Uuid,
    pub change_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreScopeRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptGitRestoreScope {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub context_snapshot_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptVersionDiff {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub from_revision: String,
    pub to_revision: String,
    pub kind: &'static str,
    pub content: Option<String>,
    pub byte_size: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitVersionError {
    InvalidInput,
    ManuscriptNotFound,
    NotAGitRepository,
    DirtyWorkingTree,
    NoStagedChanges,
    GitUnavailable,
}

fn map_git_error(error: GitServiceError) -> GitVersionError {
    match error {
        GitServiceError::Core(research_agent_core::git::GitError::NotRepository)
        | GitServiceError::Command(_) => GitVersionError::NotAGitRepository,
        GitServiceError::Core(research_agent_core::git::GitError::Invalid) => {
            GitVersionError::InvalidInput
        }
        GitServiceError::Path => GitVersionError::DirtyWorkingTree,
        GitServiceError::Timeout | GitServiceError::Utf8 | GitServiceError::Core(_) => {
            GitVersionError::GitUnavailable
        }
    }
}

fn validate_request(request: &CreateVersionRequest) -> Result<(), GitVersionError> {
    if request.manuscript_id.is_nil()
        || request.message.trim().is_empty()
        || request.message.len() > 4096
        || request.message.chars().any(|character| character == '\0')
    {
        return Err(GitVersionError::InvalidInput);
    }
    Ok(())
}

fn validate_manuscript_id(id: Uuid) -> Result<(), GitVersionError> {
    if id.is_nil() {
        Err(GitVersionError::InvalidInput)
    } else {
        Ok(())
    }
}

async fn repository_for_manuscript(
    store: &AppStore,
    id: Uuid,
) -> Result<(GitWorkspaceService, research_agent_core::git::GitRepository), GitVersionError> {
    validate_manuscript_id(id)?;
    let active = store
        .connection()
        .map_err(|_| GitVersionError::GitUnavailable)?
        .query_row(
            "select exists(select 1 from manuscripts where id=?1 and status='active')",
            [id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| GitVersionError::GitUnavailable)?;
    if !active {
        return Err(GitVersionError::ManuscriptNotFound);
    }
    let source_directory = store
        .writing_source_directory()
        .ok_or(GitVersionError::ManuscriptNotFound)?;
    let source = source_directory(store.manuscript_directory(), &id.to_string())
        .map_err(|_| GitVersionError::ManuscriptNotFound)?;
    if !Path::new(&source).is_dir() {
        return Err(GitVersionError::NotAGitRepository);
    }
    let service = GitWorkspaceService::default();
    let repository = service
        .inspect(Path::new(&source))
        .await
        .map_err(map_git_error)?;
    Ok((service, repository))
}

async fn create_version(
    store: AppStore,
    request: CreateVersionRequest,
) -> Result<ManuscriptVersion, GitVersionError> {
    validate_request(&request)?;
    let id = request.manuscript_id;
    let (service, repository) = repository_for_manuscript(&store, id).await?;
    let commit = service
        .commit_staged(&repository, &request.message)
        .await
        .map_err(|error| match error {
            GitServiceError::Path => GitVersionError::DirtyWorkingTree,
            other => map_git_error(other),
        })?;
    Ok(ManuscriptVersion {
        schema_version: 1,
        manuscript_id: id,
        commit,
    })
}

async fn list_versions(
    store: AppStore,
    request: ListVersionsRequest,
) -> Result<ManuscriptVersionHistory, GitVersionError> {
    let limit = request.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(GitVersionError::InvalidInput);
    }
    let (service, repository) = repository_for_manuscript(&store, request.manuscript_id).await?;
    let commits = service
        .log(&repository, limit.min(200))
        .await
        .map_err(map_git_error)?;
    Ok(ManuscriptVersionHistory {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        commits,
    })
}

async fn list_branches(
    store: AppStore,
    manuscript_id: Uuid,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    let (service, repository) = repository_for_manuscript(&store, manuscript_id).await?;
    let branches = service
        .list_branches(&repository)
        .await
        .map_err(map_git_error)?;
    Ok(ManuscriptGitBranches {
        schema_version: 1,
        manuscript_id,
        current: repository.branch,
        branches,
    })
}

async fn create_branch(
    store: AppStore,
    request: BranchRequest,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    if request.branch.trim().is_empty() {
        return Err(GitVersionError::InvalidInput);
    }
    let (service, repository) = repository_for_manuscript(&store, request.manuscript_id).await?;
    service
        .create_branch(&repository, &request.branch, "HEAD")
        .await
        .map_err(map_git_error)?;
    list_branches(store, request.manuscript_id).await
}

async fn delete_branch(
    store: AppStore,
    request: BranchRequest,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    if request.branch.trim().is_empty() {
        return Err(GitVersionError::InvalidInput);
    }
    let (service, repository) = repository_for_manuscript(&store, request.manuscript_id).await?;
    service
        .delete_branch(&repository, &request.branch)
        .await
        .map_err(map_git_error)?;
    list_branches(store, request.manuscript_id).await
}

async fn diff_versions(
    store: AppStore,
    request: DiffVersionsRequest,
) -> Result<ManuscriptVersionDiff, GitVersionError> {
    let (service, repository) = repository_for_manuscript(&store, request.manuscript_id).await?;
    let bytes = service
        .diff_revisions(&repository, &request.from_revision, &request.to_revision)
        .await
        .map_err(map_git_error)?;
    let byte_size = bytes.len();
    let (kind, content) = match String::from_utf8(bytes) {
        Ok(text) => ("text", Some(text)),
        Err(_) => ("binary", None),
    };
    Ok(ManuscriptVersionDiff {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        from_revision: request.from_revision,
        to_revision: request.to_revision,
        kind,
        content,
        byte_size,
    })
}

async fn restore_version(
    store: AppStore,
    request: RestoreVersionRequest,
) -> Result<ManuscriptGitRestoreProposal, GitVersionError> {
    if [
        request.conversation_id,
        request.request_id,
        request.manuscript_id,
        request.context_snapshot_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        || request.revision.trim().is_empty()
    {
        return Err(GitVersionError::InvalidInput);
    }
    let (git, repository) = repository_for_manuscript(&store, request.manuscript_id).await?;
    let snapshot = git
        .snapshot_revision(&repository, &request.revision)
        .await
        .map_err(map_git_error)?;
    let service =
        crate::agent::writing_submission::WritingSubmissionService::from_app_store(&store);
    let review = service
        .review_workspace(
            crate::agent::writing_submission::WritingSubmissionReceipt {
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                manuscript_id: request.manuscript_id,
            },
            request.context_snapshot_id,
        )
        .await
        .map_err(|_| GitVersionError::GitUnavailable)?;
    let baseline = review
        .baseline
        .files
        .iter()
        .map(|entry| entry.file.clone())
        .collect::<Vec<WorkspaceFile>>();
    let mut changes = super::git::revision_changes(&baseline, &snapshot).map_err(map_git_error)?;
    let sources = review
        .baseline
        .files
        .iter()
        .map(|entry| (entry.file.path.as_str(), entry.source.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for change in &mut changes {
        change.source = sources.get(change.path.as_str()).cloned();
    }
    if changes.is_empty() {
        return Err(GitVersionError::InvalidInput);
    }
    let payloads = snapshot
        .into_iter()
        .map(|file| {
            (
                format!("{:x}", sha2::Sha256::digest(&file.bytes)),
                file.bytes,
            )
        })
        .collect();
    let change_count = changes.len();
    let proposal_id = service
        .publish_external_changes(review.binding, changes, payloads)
        .await
        .map_err(|_| GitVersionError::GitUnavailable)?;
    Ok(ManuscriptGitRestoreProposal {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        revision: request.revision,
        proposal_id,
        change_count,
    })
}

async fn latest_restore_scope(
    store: AppStore,
    request: RestoreScopeRequest,
) -> Result<Option<ManuscriptGitRestoreScope>, GitVersionError> {
    if request.conversation_id.is_nil() || request.manuscript_id.is_nil() {
        return Err(GitVersionError::InvalidInput);
    }
    let connection = store
        .connection()
        .map_err(|_| GitVersionError::GitUnavailable)?;
    let value =
        latest_restore_scope_in(&connection, request.conversation_id, request.manuscript_id)
            .map_err(|_| GitVersionError::GitUnavailable)?;
    Ok(value.map(
        |(request_id, context_snapshot_id)| ManuscriptGitRestoreScope {
            schema_version: 1,
            conversation_id: request.conversation_id,
            manuscript_id: request.manuscript_id,
            request_id,
            context_snapshot_id,
        },
    ))
}

fn latest_restore_scope_in(
    connection: &Connection,
    conversation_id: Uuid,
    manuscript_id: Uuid,
) -> Result<Option<(Uuid, Uuid)>, rusqlite::Error> {
    connection
        .query_row(
            "select request_id,context_snapshot_id from agent_execution_workspaces
             where conversation_id=?1 and manuscript_id=?2
               and state in ('prepared', 'approved')
             order by rowid desc limit 1",
            rusqlite::params![conversation_id.to_string(), manuscript_id.to_string()],
            |row| {
                let request_id = Uuid::parse_str(&row.get::<_, String>(0)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let context_snapshot_id = Uuid::parse_str(&row.get::<_, String>(1)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok((request_id, context_snapshot_id))
            },
        )
        .optional()
}

async fn initialize(
    store: AppStore,
    request: InitializeRequest,
) -> Result<GitRepositoryView, GitVersionError> {
    let repository = {
        validate_manuscript_id(request.manuscript_id)?;
        let active = store
            .connection()
            .map_err(|_| GitVersionError::GitUnavailable)?
            .query_row(
                "select exists(select 1 from manuscripts where id=?1 and status='active')",
                [request.manuscript_id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| GitVersionError::GitUnavailable)?;
        if !active {
            return Err(GitVersionError::ManuscriptNotFound);
        }
        let source_directory = store
            .writing_source_directory()
            .ok_or(GitVersionError::ManuscriptNotFound)?;
        let source = source_directory(
            store.manuscript_directory(),
            &request.manuscript_id.to_string(),
        )
        .map_err(|_| GitVersionError::ManuscriptNotFound)?;
        GitWorkspaceService::default()
            .initialize(Path::new(&source))
            .await
            .map_err(map_git_error)?
    };
    Ok(GitRepositoryView {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        root: repository.root,
        branch: repository.branch,
        head: repository.head.map(|head| head.as_str().to_owned()),
    })
}

#[tauri::command]
pub(crate) async fn agent_create_manuscript_git_version(
    store: State<'_, AppStore>,
    request: CreateVersionRequest,
) -> Result<ManuscriptVersion, GitVersionError> {
    create_version(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_list_manuscript_git_versions(
    store: State<'_, AppStore>,
    request: ListVersionsRequest,
) -> Result<ManuscriptVersionHistory, GitVersionError> {
    list_versions(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_initialize_manuscript_git_repository(
    store: State<'_, AppStore>,
    request: InitializeRequest,
) -> Result<GitRepositoryView, GitVersionError> {
    initialize(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_list_manuscript_git_branches(
    store: State<'_, AppStore>,
    manuscript_id: Uuid,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    list_branches(store.inner().clone(), manuscript_id).await
}

#[tauri::command]
pub(crate) async fn agent_create_manuscript_git_branch(
    store: State<'_, AppStore>,
    request: BranchRequest,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    create_branch(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_delete_manuscript_git_branch(
    store: State<'_, AppStore>,
    request: BranchRequest,
) -> Result<ManuscriptGitBranches, GitVersionError> {
    delete_branch(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_diff_manuscript_git_versions(
    store: State<'_, AppStore>,
    request: DiffVersionsRequest,
) -> Result<ManuscriptVersionDiff, GitVersionError> {
    diff_versions(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_restore_manuscript_git_version(
    store: State<'_, AppStore>,
    request: RestoreVersionRequest,
) -> Result<ManuscriptGitRestoreProposal, GitVersionError> {
    restore_version(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_latest_manuscript_git_restore_scope(
    store: State<'_, AppStore>,
    request: RestoreScopeRequest,
) -> Result<Option<ManuscriptGitRestoreScope>, GitVersionError> {
    latest_restore_scope(store.inner().clone(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_and_lookup_preserve_input_database_and_active_order() {
        let root = tempfile::tempdir().unwrap();
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        let id = Uuid::new_v4();
        assert!(matches!(
            initialize(
                store.clone(),
                InitializeRequest {
                    manuscript_id: Uuid::nil()
                }
            )
            .await,
            Err(GitVersionError::InvalidInput)
        ));
        assert!(matches!(
            repository_for_manuscript(&store, Uuid::nil()).await,
            Err(GitVersionError::InvalidInput)
        ));
        assert!(matches!(
            initialize(store.clone(), InitializeRequest { manuscript_id: id }).await,
            Err(GitVersionError::GitUnavailable)
        ));
        assert!(matches!(
            repository_for_manuscript(&store, id).await,
            Err(GitVersionError::GitUnavailable)
        ));
        store
            .connection()
            .unwrap()
            .execute_batch("create table manuscripts(id text,status text)")
            .unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "insert into manuscripts values(?1,'archived')",
                [id.to_string()],
            )
            .unwrap();
        let source = (store.writing_resource_source().unwrap().source_directory)(
            store.manuscript_directory(),
            &id.to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(&source).unwrap();
        assert!(matches!(
            initialize(store.clone(), InitializeRequest { manuscript_id: id }).await,
            Err(GitVersionError::ManuscriptNotFound)
        ));
        assert!(matches!(
            repository_for_manuscript(&store, id).await,
            Err(GitVersionError::ManuscriptNotFound)
        ));
        assert!(!source.join(".git").exists());
        store
            .connection()
            .unwrap()
            .execute("update manuscripts set status='active'", [])
            .unwrap();
        let initialized = initialize(store.clone(), InitializeRequest { manuscript_id: id })
            .await
            .unwrap();
        let (_, repository) = repository_for_manuscript(&store, id).await.unwrap();
        assert_eq!(repository.root, initialized.root);
        assert_eq!(
            repository.root,
            source
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        store
            .connection()
            .unwrap()
            .execute("update manuscripts set status='archived'", [])
            .unwrap();
        assert!(matches!(
            initialize(store.clone(), InitializeRequest { manuscript_id: id }).await,
            Err(GitVersionError::ManuscriptNotFound)
        ));
        assert!(matches!(
            repository_for_manuscript(&store, id).await,
            Err(GitVersionError::ManuscriptNotFound)
        ));
    }

    #[test]
    fn request_rejects_empty_or_overlong_messages() {
        assert!(
            validate_request(&CreateVersionRequest {
                manuscript_id: Uuid::new_v4(),
                message: "  ".into(),
            })
            .is_err()
        );
        assert!(
            validate_request(&CreateVersionRequest {
                manuscript_id: Uuid::new_v4(),
                message: "x".repeat(4097),
            })
            .is_err()
        );
        assert!(
            validate_request(&CreateVersionRequest {
                manuscript_id: Uuid::new_v4(),
                message: "version".into(),
            })
            .is_ok()
        );
    }

    #[test]
    fn restore_scope_ignores_revoked_workspaces() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "create table agent_execution_workspaces(
                conversation_id text not null, request_id text not null,
                manuscript_id text not null, context_snapshot_id text not null,
                state text not null
            );",
        )
        .unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let revoked_request = Uuid::new_v4();
        let revoked_snapshot = Uuid::new_v4();
        db.execute(
            "insert into agent_execution_workspaces values (?1,?2,?3,?4,'revoked')",
            rusqlite::params![
                conversation.to_string(),
                revoked_request.to_string(),
                manuscript.to_string(),
                revoked_snapshot.to_string()
            ],
        )
        .unwrap();
        let active_request = Uuid::new_v4();
        let active_snapshot = Uuid::new_v4();
        db.execute(
            "insert into agent_execution_workspaces values (?1,?2,?3,?4,'approved')",
            rusqlite::params![
                conversation.to_string(),
                active_request.to_string(),
                manuscript.to_string(),
                active_snapshot.to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            latest_restore_scope_in(&db, conversation, manuscript).unwrap(),
            Some((active_request, active_snapshot))
        );
    }
}
