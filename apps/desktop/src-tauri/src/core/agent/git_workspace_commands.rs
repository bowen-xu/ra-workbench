//! Explicit lifecycle commands for application-owned Git worktrees.
//! Paths are always derived from the manuscript and durable registry; callers
//! cannot select an arbitrary directory to create or remove.

use super::{
    git::{GitServiceError, GitWorkspaceService},
    repository::git_workspaces::{self, Record, State},
};
use crate::storage::AppStore;
use research_agent_core::git::{GitRepository, GitWorktree};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use tauri::State as TauriState;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRequest {
    pub manuscript_id: Uuid,
    pub revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRequest {
    pub manuscript_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveRequest {
    pub manuscript_id: Uuid,
    pub worktree_id: Uuid,
    pub discard_changes: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeView {
    pub schema_version: u8,
    pub id: Uuid,
    pub manuscript_id: Uuid,
    pub repository_root: String,
    pub worktree_path: String,
    pub revision: String,
    pub state: String,
    pub git: Option<GitWorktree>,
    pub status: Option<WorktreeStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    pub dirty: bool,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeError {
    InvalidInput,
    ManuscriptNotFound,
    NotAGitRepository,
    WorktreeNotFound,
    DirtyWorktree,
    RecoveryRequired,
    Conflict,
    GitUnavailable,
}

fn map_git(error: GitServiceError) -> WorktreeError {
    match error {
        GitServiceError::Path => WorktreeError::Conflict,
        GitServiceError::Core(research_agent_core::git::GitError::NotRepository)
        | GitServiceError::Command(_) => WorktreeError::NotAGitRepository,
        GitServiceError::Core(_) | GitServiceError::Timeout | GitServiceError::Utf8 => {
            WorktreeError::GitUnavailable
        }
    }
}

fn view(record: Record, git: Option<GitWorktree>, status: Option<WorktreeStatus>) -> WorktreeView {
    WorktreeView {
        schema_version: 1,
        id: record.id,
        manuscript_id: record.manuscript_id,
        repository_root: record.repository_root,
        worktree_path: record.worktree_path,
        revision: record.revision,
        state: match record.state {
            State::Active => "active",
            State::Released => "released",
            State::RecoveryRequired => "recovery_required",
        }
        .into(),
        git,
        status,
    }
}

fn active_manuscript(store: &AppStore, id: Uuid) -> Result<(), WorktreeError> {
    if id.is_nil() {
        return Err(WorktreeError::InvalidInput);
    }
    let active = store
        .connection()
        .map_err(|_| WorktreeError::GitUnavailable)?
        .query_row(
            "select exists(select 1 from manuscripts where id=?1 and status='active')",
            [id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| WorktreeError::GitUnavailable)?;
    if active {
        Ok(())
    } else {
        Err(WorktreeError::ManuscriptNotFound)
    }
}

async fn create(store: AppStore, request: CreateRequest) -> Result<WorktreeView, WorktreeError> {
    active_manuscript(&store, request.manuscript_id)?;
    let source_directory = store
        .writing_source_directory()
        .ok_or(WorktreeError::ManuscriptNotFound)?;
    let source = source_directory(
        store.manuscript_directory(),
        &request.manuscript_id.to_string(),
    )
    .map_err(|_| WorktreeError::ManuscriptNotFound)?;
    let service = GitWorkspaceService::default();
    let repository = service.inspect(Path::new(&source)).await.map_err(map_git)?;
    let id = Uuid::new_v4();
    let revision = request.revision.unwrap_or_else(|| "HEAD".into());
    let root = store
        .agent_workspace_directory()
        .join("git")
        .join(request.manuscript_id.to_string());
    fs::create_dir_all(&root).map_err(|_| WorktreeError::GitUnavailable)?;
    let target = root.join(id.to_string());
    let path = service
        .create_worktree(&repository, &target, &revision)
        .await
        .map_err(map_git)?;
    let record = Record {
        id,
        manuscript_id: request.manuscript_id,
        repository_root: repository.root.clone(),
        worktree_path: path.to_string_lossy().into_owned(),
        revision,
        state: State::Active,
    };
    let insert_result = {
        let mut db = store
            .connection()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        let tx = db
            .transaction()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        let result = git_workspaces::insert_in(&tx, &record);
        let commit = if result.is_ok() {
            tx.commit().map_err(|_| WorktreeError::GitUnavailable)
        } else {
            Ok(())
        };
        match (result, commit) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(match error {
                git_workspaces::Error::Conflict => WorktreeError::Conflict,
                git_workspaces::Error::Storage => WorktreeError::GitUnavailable,
            }),
            (_, Err(error)) => Err(error),
        }
    };
    if insert_result.is_err() {
        // The worktree was just created and is not exposed in the registry;
        // cleanup is safe and prevents an orphaned application path.
        let _ = service.remove_worktree(&repository, &path, true).await;
        return Err(WorktreeError::Conflict);
    }
    let git = service.open_worktree(&repository, &path).await.ok();
    let status = worktree_status(&service, &repository, git.as_ref()).await;
    Ok(view(record, git, status))
}

async fn list(store: AppStore, request: ListRequest) -> Result<Vec<WorktreeView>, WorktreeError> {
    active_manuscript(&store, request.manuscript_id)?;
    let records = {
        let db = store
            .connection()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        git_workspaces::list(&db, request.manuscript_id)
            .map_err(|_| WorktreeError::GitUnavailable)?
    };
    let service = GitWorkspaceService::default();
    let mut output = Vec::with_capacity(records.len());
    for mut record in records {
        let repository = GitRepository {
            root: record.repository_root.clone(),
            git_dir: String::new(),
            worktree: Some(record.repository_root.clone()),
            head: None,
            branch: None,
            remotes: Default::default(),
        };
        let git = service
            .open_worktree(&repository, Path::new(&record.worktree_path))
            .await
            .ok();
        let status = worktree_status(&service, &repository, git.as_ref()).await;
        if git.is_none() && record.state == State::Active {
            record.state = State::RecoveryRequired;
            if let Ok(mut db) = store.connection()
                && let Ok(tx) = db.transaction()
            {
                let _ = git_workspaces::mark_recovery_required_in(&tx, record.id);
                let _ = tx.commit();
            }
        }
        output.push(view(record, git, status));
    }
    Ok(output)
}

async fn worktree_status(
    service: &GitWorkspaceService,
    repository: &GitRepository,
    worktree: Option<&GitWorktree>,
) -> Option<WorktreeStatus> {
    let worktree = worktree?;
    let status_repository = GitRepository {
        root: worktree.path.clone(),
        git_dir: repository.git_dir.clone(),
        worktree: Some(worktree.path.clone()),
        head: worktree.head.clone(),
        branch: worktree.branch.clone(),
        remotes: repository.remotes.clone(),
    };
    let (_, entries) = service.status(&status_repository).await.ok()?;
    let staged_count = entries.iter().filter(|entry| entry.staged).count();
    let unstaged_count = entries.iter().filter(|entry| entry.unstaged).count();
    let untracked_count = entries
        .iter()
        .filter(|entry| entry.kind == research_agent_core::git::GitChangeKind::Untracked)
        .count();
    Some(WorktreeStatus {
        dirty: !entries.is_empty(),
        staged_count,
        unstaged_count,
        untracked_count,
    })
}

async fn remove(store: AppStore, request: RemoveRequest) -> Result<(), WorktreeError> {
    active_manuscript(&store, request.manuscript_id)?;
    let record = {
        let db = store
            .connection()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        git_workspaces::read(&db, request.manuscript_id, request.worktree_id)
            .map_err(|_| WorktreeError::GitUnavailable)?
            .ok_or(WorktreeError::WorktreeNotFound)?
    };
    if record.state == State::Released {
        return Ok(());
    }
    if record.state == State::RecoveryRequired && !request.discard_changes {
        return Err(WorktreeError::RecoveryRequired);
    }
    // A startup reconciliation may find that the disposable worktree was
    // already removed outside the app. With an explicit discard confirmation
    // there is no Git object left to clean; release only the durable inventory
    // row instead of attempting a second removal and reporting a false error.
    if record.state == State::RecoveryRequired
        && request.discard_changes
        && !Path::new(&record.worktree_path).exists()
    {
        let mut db = store
            .connection()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        let tx = db
            .transaction()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        git_workspaces::release_in(&tx, record.id).map_err(|_| WorktreeError::Conflict)?;
        tx.commit().map_err(|_| WorktreeError::GitUnavailable)?;
        return Ok(());
    }
    let service = GitWorkspaceService::default();
    let repository = service
        .inspect(Path::new(&record.repository_root))
        .await
        .map_err(map_git)?;
    let worktree = service
        .open_worktree(&repository, Path::new(&record.worktree_path))
        .await
        .map_err(|_| WorktreeError::RecoveryRequired)?;
    let (_, entries) = service
        .status(&GitRepository {
            root: worktree.path.clone(),
            git_dir: repository.git_dir.clone(),
            worktree: Some(worktree.path.clone()),
            head: worktree.head.clone(),
            branch: worktree.branch.clone(),
            remotes: Default::default(),
        })
        .await
        .map_err(map_git)?;
    if !entries.is_empty() && !request.discard_changes {
        return Err(WorktreeError::DirtyWorktree);
    }
    service
        .remove_worktree(
            &repository,
            Path::new(&record.worktree_path),
            request.discard_changes,
        )
        .await
        .map_err(map_git)?;
    let mut db = store
        .connection()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    let tx = db
        .transaction()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    git_workspaces::release_in(&tx, record.id).map_err(|_| WorktreeError::Conflict)?;
    tx.commit().map_err(|_| WorktreeError::GitUnavailable)
}

#[tauri::command]
pub(crate) async fn agent_create_git_worktree(
    store: TauriState<'_, AppStore>,
    request: CreateRequest,
) -> Result<WorktreeView, WorktreeError> {
    create(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_list_git_worktrees(
    store: TauriState<'_, AppStore>,
    request: ListRequest,
) -> Result<Vec<WorktreeView>, WorktreeError> {
    list(store.inner().clone(), request).await
}

#[tauri::command]
pub(crate) async fn agent_remove_git_worktree(
    store: TauriState<'_, AppStore>,
    request: RemoveRequest,
) -> Result<(), WorktreeError> {
    remove(store.inner().clone(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::{path::PathBuf, process::Command};
    use tempfile::TempDir;

    struct Fixture {
        _root: TempDir,
        store: AppStore,
        manuscript: Uuid,
        source: PathBuf,
    }

    fn git(directory: &Path, arguments: &[&str]) {
        assert!(
            Command::new("git")
                .current_dir(directory)
                .args(arguments)
                .status()
                .unwrap()
                .success(),
            "git command failed: {arguments:?}"
        );
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let mut database = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut database, 98);
        let manuscript = Uuid::new_v4();
        database
            .execute(
                "insert into research_projects(id,title,summary,created_at) values ('project','fixture','','now')",
                [],
            )
            .unwrap();
        database
            .execute(
                "insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','fixture','main.tex','now','now')",
                [manuscript.to_string()],
            )
            .unwrap();
        let store = AppStore::for_test(database, root.path().join("data/objects"));
        let source = (store.writing_resource_source().unwrap().source_directory)(
            store.manuscript_directory(),
            &manuscript.to_string(),
        )
        .unwrap();
        fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "--initial-branch=main"]);
        git(&source, &["config", "user.name", "Test"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        fs::write(source.join("main.tex"), "initial\n").unwrap();
        git(&source, &["add", "main.tex"]);
        git(&source, &["commit", "-m", "initial"]);
        Fixture {
            _root: root,
            store,
            manuscript,
            source,
        }
    }

    #[tokio::test]
    async fn lifecycle_starts_clean_reports_dirty_state_and_requires_discard() {
        let fixture = fixture();
        let created = create(
            fixture.store.clone(),
            CreateRequest {
                manuscript_id: fixture.manuscript,
                revision: None,
            },
        )
        .await
        .unwrap();
        let path = PathBuf::from(&created.worktree_path);
        assert_eq!(fs::read(path.join("main.tex")).unwrap(), b"initial\n");
        assert!(!created.status.as_ref().unwrap().dirty);

        fs::write(path.join("main.tex"), "staged\n").unwrap();
        git(&path, &["add", "main.tex"]);
        fs::write(path.join("main.tex"), "staged and unstaged\n").unwrap();
        fs::write(path.join("scratch.txt"), "untracked\n").unwrap();

        let entries = list(
            fixture.store.clone(),
            ListRequest {
                manuscript_id: fixture.manuscript,
            },
        )
        .await
        .unwrap();
        let status = entries[0].status.as_ref().unwrap();
        assert!(status.dirty);
        assert_eq!(status.staged_count, 1);
        // Git reports the modified file and the untracked file as unstaged;
        // the latter is also exposed separately by `untracked_count`.
        assert_eq!(status.unstaged_count, 2);
        assert_eq!(status.untracked_count, 1);

        assert!(matches!(
            remove(
                fixture.store.clone(),
                RemoveRequest {
                    manuscript_id: fixture.manuscript,
                    worktree_id: created.id,
                    discard_changes: false,
                },
            )
            .await,
            Err(WorktreeError::DirtyWorktree)
        ));
        assert!(path.is_dir());
        remove(
            fixture.store.clone(),
            RemoveRequest {
                manuscript_id: fixture.manuscript,
                worktree_id: created.id,
                discard_changes: true,
            },
        )
        .await
        .unwrap();
        assert!(!path.exists());
        remove(
            fixture.store.clone(),
            RemoveRequest {
                manuscript_id: fixture.manuscript,
                worktree_id: created.id,
                discard_changes: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            list(
                fixture.store,
                ListRequest {
                    manuscript_id: fixture.manuscript,
                },
            )
            .await
            .unwrap()[0]
                .state,
            "released"
        );
    }

    #[tokio::test]
    async fn missing_worktree_requires_explicit_recovery_without_deleting_other_paths() {
        let fixture = fixture();
        let created = create(
            fixture.store.clone(),
            CreateRequest {
                manuscript_id: fixture.manuscript,
                revision: None,
            },
        )
        .await
        .unwrap();
        let path = PathBuf::from(&created.worktree_path);
        git(
            &fixture.source,
            &["worktree", "remove", "--force", path.to_str().unwrap()],
        );
        assert!(!path.exists());

        let listed = list(
            fixture.store.clone(),
            ListRequest {
                manuscript_id: fixture.manuscript,
            },
        )
        .await
        .unwrap();
        assert_eq!(listed[0].state, "recovery_required");
        assert!(matches!(
            remove(
                fixture.store.clone(),
                RemoveRequest {
                    manuscript_id: fixture.manuscript,
                    worktree_id: created.id,
                    discard_changes: false,
                },
            )
            .await,
            Err(WorktreeError::RecoveryRequired)
        ));

        let unrelated = fixture.source.parent().unwrap().join("unrelated");
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(unrelated.join("keep.txt"), "keep").unwrap();
        remove(
            fixture.store,
            RemoveRequest {
                manuscript_id: fixture.manuscript,
                worktree_id: created.id,
                discard_changes: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(fs::read(unrelated.join("keep.txt")).unwrap(), b"keep");
    }
}
