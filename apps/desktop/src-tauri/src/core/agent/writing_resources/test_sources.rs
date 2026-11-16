//! Caller-owned test snapshots with canonical Writing Native rules and resources.
use super::{WritingContextError, WritingResourceFile, WritingResourceSource};
use rusqlite::Connection;

pub(crate) fn source() -> WritingResourceSource {
    WritingResourceSource {
        source_directory: super::source_directory,
        is_text_path: research_writing_projects_native::projects::is_text_path,
        list_host_files,
        list_text_paths,
    }
}

fn list_host_files(
    db: &Connection,
    manuscript: &str,
) -> Result<Vec<WritingResourceFile>, WritingContextError> {
    let mut query = db
        .prepare(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/context-resources-v1/writing_projects/project-files.v1.sql"
        )))
        .map_err(|_| WritingContextError::Unavailable)?;
    query
        .query_map([manuscript], |row| {
            Ok(WritingResourceFile {
                relative_path: row.get(1)?,
                kind: row.get(2)?,
                sha256: row.get(4)?,
            })
        })
        .map_err(|_| WritingContextError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| WritingContextError::Unavailable)
}

fn list_text_paths(db: &Connection, manuscript: &str) -> Result<Vec<String>, WritingContextError> {
    let mut query = db
        .prepare(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/context-resources-v1/writing_projects/project-context-text-paths.v1.sql"
        )))
        .map_err(|_| WritingContextError::Unavailable)?;
    query
        .query_map([manuscript], |row| row.get(0))
        .map_err(|_| WritingContextError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| WritingContextError::Unavailable)
}
