use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;
use uuid::Uuid;

use crate::storage::{AppStore, StorageError};

#[derive(Debug, Error)]
pub(crate) enum ProjectError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("项目数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("项目名称不能为空")]
    EmptyTitle,
    #[error("项目名称不能超过 120 个字符")]
    TitleTooLong,
    #[error("项目说明不能超过 600 个字符")]
    SummaryTooLong,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    code: &'static str,
    message: String,
}

impl From<ProjectError> for CommandError {
    fn from(error: ProjectError) -> Self {
        let code = match error {
            ProjectError::EmptyTitle
            | ProjectError::TitleTooLong
            | ProjectError::SummaryTooLong => "invalid_project",
            ProjectError::Storage(_) | ProjectError::Database(_) => "storage_error",
        };

        Self {
            code,
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDraft {
    title: String,
    summary: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResearchProject {
    id: String,
    title: String,
    summary: String,
    created_at: String,
    literature_count: i64,
    evidence_count: i64,
}

fn validate_draft(draft: ProjectDraft) -> Result<ProjectDraft, ProjectError> {
    let title = draft.title.trim().to_owned();
    let summary = draft.summary.trim().to_owned();

    if title.is_empty() {
        return Err(ProjectError::EmptyTitle);
    }
    if title.chars().count() > 120 {
        return Err(ProjectError::TitleTooLong);
    }
    if summary.chars().count() > 600 {
        return Err(ProjectError::SummaryTooLong);
    }

    Ok(ProjectDraft { title, summary })
}

fn list_from(connection: &Connection) -> Result<Vec<ResearchProject>, ProjectError> {
    let mut statement = connection.prepare(
        "select p.id, p.title, p.summary, p.created_at,
                (select count(*) from project_items pi join bibliographic_items b on b.id = pi.item_id
                 where pi.project_id = p.id and b.archived_at is null),
                (select count(*) from evidence_links e join claims c on c.id = e.claim_id
                 where c.project_id = p.id)
         from research_projects p order by p.created_at desc, p.id desc",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ResearchProject {
            id: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            created_at: row.get(3)?,
            literature_count: row.get(4)?,
            evidence_count: row.get(5)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn create_in(
    connection: &Connection,
    draft: ProjectDraft,
) -> Result<ResearchProject, ProjectError> {
    let draft = validate_draft(draft)?;
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "insert into research_projects (id, title, summary, created_at)
         values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![id, draft.title, draft.summary],
    )?;

    connection
        .query_row(
            "select id, title, summary, created_at, 0, 0
             from research_projects where id = ?1",
            [id],
            |row| {
                Ok(ResearchProject {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    summary: row.get(2)?,
                    created_at: row.get(3)?,
                    literature_count: row.get(4)?,
                    evidence_count: row.get(5)?,
                })
            },
        )
        .map_err(Into::into)
}

#[tauri::command]
pub fn list_projects(store: State<'_, AppStore>) -> Result<Vec<ResearchProject>, CommandError> {
    let connection = store.connection().map_err(ProjectError::from)?;
    list_from(&connection).map_err(Into::into)
}

#[tauri::command]
pub fn create_project(
    store: State<'_, AppStore>,
    draft: ProjectDraft,
) -> Result<ResearchProject, CommandError> {
    let connection = store.connection().map_err(ProjectError::from)?;
    create_in(&connection, draft).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        crate::storage::migrate(&mut connection).expect("run migrations");
        connection
    }

    #[test]
    fn creates_and_lists_a_project() {
        let connection = test_connection();
        let created = create_in(
            &connection,
            ProjectDraft {
                title: "  可追溯问答  ".to_owned(),
                summary: "  验证来源锚点  ".to_owned(),
            },
        )
        .expect("create project");

        assert_eq!(created.title, "可追溯问答");
        assert_eq!(
            list_from(&connection).expect("list projects"),
            vec![created]
        );
    }

    #[test]
    fn rejects_blank_titles() {
        let connection = test_connection();
        let result = create_in(
            &connection,
            ProjectDraft {
                title: "   ".to_owned(),
                summary: String::new(),
            },
        );

        assert!(matches!(result, Err(ProjectError::EmptyTitle)));
    }

    #[test]
    fn counts_traceable_evidence_links_for_each_project() {
        let connection = test_connection();
        let project = create_in(
            &connection,
            ProjectDraft {
                title: "Evidence metrics".to_owned(),
                summary: String::new(),
            },
        )
        .expect("create project");
        let hash = "d".repeat(64);
        connection
            .execute(
                "insert into bibliographic_items
                    (id, item_type, title, created_at, updated_at)
                 values ('item', 'document', 'Source', 'now', 'now')",
                [],
            )
            .expect("item");
        connection
            .execute(
                "insert into project_items (project_id, item_id, added_at) values (?1, 'item', 'now')",
                [&project.id],
            )
            .expect("project item");
        connection
            .execute(
                "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
                 values (?1, 'dd/dd/source.pdf', 'application/pdf', 1, 'now')",
                [&hash],
            )
            .expect("content");
        connection
            .execute(
                "insert into attachments (id, item_id, content_sha256, display_name, created_at)
                 values ('attachment', 'item', ?1, 'source.pdf', 'now')",
                [&hash],
            )
            .expect("attachment");
        connection
            .execute(
                "insert into source_anchors
                    (id, item_id, attachment_id, content_sha256, page_index, rects_json, created_at)
                 values ('anchor', 'item', 'attachment', ?1, 0, '[]', 'now')",
                [&hash],
            )
            .expect("anchor");
        connection
            .execute(
                "insert into claims (id, project_id, statement, origin, created_at, updated_at)
                 values ('claim', ?1, 'Claim', 'user', 'now', 'now')",
                [&project.id],
            )
            .expect("claim");
        connection
            .execute(
                "insert into evidence_links (id, claim_id, anchor_id, stance, created_at)
                 values ('evidence', 'claim', 'anchor', 'supports', 'now')",
                [],
            )
            .expect("evidence");

        let reloaded = list_from(&connection)
            .expect("list projects")
            .into_iter()
            .find(|value| value.id == project.id)
            .expect("reloaded project");
        assert_eq!(reloaded.literature_count, 1);
        assert_eq!(reloaded.evidence_count, 1);
    }
}
