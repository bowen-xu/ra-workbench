//! Read-only historical Git version previews.

use super::git::{GitServiceError, GitWorkspaceService};
use crate::storage::AppStore;
use research_agent_core::git::GitSha;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadFileRequest {
    pub manuscript_id: Uuid,
    pub revision: String,
    pub relative_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalFile {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub revision: String,
    pub relative_path: String,
    pub kind: &'static str,
    pub content: Option<String>,
    pub byte_size: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalFileError {
    InvalidInput,
    ManuscriptNotFound,
    NotAGitRepository,
    FileNotFound,
    GitUnavailable,
}

async fn read_file(
    store: AppStore,
    request: ReadFileRequest,
) -> Result<HistoricalFile, HistoricalFileError> {
    if request.manuscript_id.is_nil()
        || GitSha::new(&request.revision).is_err()
        || request.relative_path.len() > 4096
    {
        return Err(HistoricalFileError::InvalidInput);
    }
    let source_directory = store
        .writing_source_directory()
        .ok_or(HistoricalFileError::ManuscriptNotFound)?;
    let source = source_directory(
        store.manuscript_directory(),
        &request.manuscript_id.to_string(),
    )
    .map_err(|_| HistoricalFileError::ManuscriptNotFound)?;
    let service = GitWorkspaceService::default();
    let repository = service
        .inspect(Path::new(&source))
        .await
        .map_err(|error| match error {
            GitServiceError::Path | GitServiceError::Command(_) => {
                HistoricalFileError::NotAGitRepository
            }
            GitServiceError::Core(_) | GitServiceError::Timeout | GitServiceError::Utf8 => {
                HistoricalFileError::GitUnavailable
            }
        })?;
    let bytes = service
        .show_file(&repository, &request.revision, &request.relative_path)
        .await
        .map_err(|error| match error {
            GitServiceError::Command(_) => HistoricalFileError::FileNotFound,
            _ => HistoricalFileError::GitUnavailable,
        })?;
    let byte_size = bytes.len();
    let (kind, content) = match String::from_utf8(bytes) {
        Ok(content) => ("text", Some(content)),
        Err(_) => ("binary", None),
    };
    Ok(HistoricalFile {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        revision: request.revision,
        relative_path: request.relative_path,
        kind,
        content,
        byte_size,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_manuscript_git_version_file(
    store: State<'_, AppStore>,
    request: ReadFileRequest,
) -> Result<HistoricalFile, HistoricalFileError> {
    read_file(store.inner().clone(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[tokio::test]
    async fn historical_read_uses_the_fixed_commit_without_an_active_manuscript_query() {
        let root = tempfile::tempdir().unwrap();
        // Historical reads never required an active manuscript query. An empty
        // database makes any newly added authorization lookup observable.
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        let manuscript = Uuid::new_v4();
        let source = (store.writing_resource_source().unwrap().source_directory)(
            store.manuscript_directory(),
            &manuscript.to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(&source).unwrap();
        let request = |revision: &str, path: &str| ReadFileRequest {
            manuscript_id: manuscript,
            revision: revision.into(),
            relative_path: path.into(),
        };
        assert!(matches!(
            read_file(store.clone(), request("invalid", "main.tex")).await,
            Err(HistoricalFileError::InvalidInput)
        ));
        assert!(matches!(
            read_file(store.clone(), request(&"a".repeat(40), "main.tex")).await,
            Err(HistoricalFileError::NotAGitRepository)
        ));
        let service = GitWorkspaceService::default();
        service.initialize(&source).await.unwrap();
        std::fs::write(source.join("main.tex"), b"snapshot\n").unwrap();
        for arguments in [
            vec!["add", "main.tex"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "snapshot",
            ],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(&source)
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let revision = service
            .inspect(&source)
            .await
            .unwrap()
            .head
            .unwrap()
            .as_str()
            .to_owned();
        std::fs::write(source.join("main.tex"), b"later input\n").unwrap();
        let file = read_file(store.clone(), request(&revision, "main.tex"))
            .await
            .unwrap();
        assert_eq!(file.manuscript_id, manuscript);
        assert_eq!(file.revision, revision);
        assert_eq!(file.relative_path, "main.tex");
        assert_eq!(file.kind, "text");
        assert_eq!(file.content.as_deref(), Some("snapshot\n"));
        assert_eq!(file.byte_size, 9);
        assert!(matches!(
            read_file(store, request(&revision, "missing.tex")).await,
            Err(HistoricalFileError::FileNotFound)
        ));
    }
}
