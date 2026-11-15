use rusqlite::params;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::storage::AppStore;

use super::error::{NotesCommandError, NotesError};

#[derive(Debug, Serialize)]
pub struct NoteTag {
    id: String,
    name: String,
}

#[tauri::command]
pub fn list_note_tags(store: State<'_, AppStore>) -> Result<Vec<NoteTag>, NotesCommandError> {
    let connection = store.connection()?;
    let mut statement =
        connection.prepare("select id, name from tags order by name collate nocase")?;
    Ok(statement
        .query_map([], |row| {
            Ok(NoteTag {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

#[tauri::command]
pub fn ensure_note_tag(
    store: State<'_, AppStore>,
    name: String,
) -> Result<NoteTag, NotesCommandError> {
    let normalized = name.trim();
    if normalized.is_empty() || normalized.chars().count() > 80 {
        return Err(NotesError::InvalidInput("标签名称无效".to_owned()).into());
    }
    let connection = store.connection()?;
    connection.execute(
        "insert into tags(id, name) values (?1, ?2) on conflict(name) do nothing",
        params![Uuid::new_v4().to_string(), normalized],
    )?;
    connection
        .query_row(
            "select id, name from tags where name = ?1 collate nocase",
            [normalized],
            |row| {
                Ok(NoteTag {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .map_err(Into::into)
}
