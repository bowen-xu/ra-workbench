use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::storage::AppStore;

use super::error::{NotesCommandError, NotesError};

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteSourceLabel {
    pub page_id: String,
    pub label: String,
}

/// Display snapshots only: no document loading or per-link target resolution.
pub(crate) fn source_labels_from(
    connection: &Connection,
    page_ids: &[String],
) -> Result<Vec<NoteSourceLabel>, NotesError> {
    if page_ids.len() > 500 {
        return Err(NotesError::InvalidInput(
            "note_source_labels_limit：一次最多读取 500 篇笔记的来源标签。".to_owned(),
        ));
    }
    if page_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = serde_json::to_string(page_ids)
        .map_err(|error| NotesError::InvalidInput(error.to_string()))?;
    let mut statement = connection.prepare(
        "select source_page_id, target_snapshot_json from note_links
         where source_page_id in (select value from json_each(?1))
           and target_kind in ('paper', 'manuscript') order by created_at",
    )?;
    let rows = statement.query_map([ids], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (id, raw) = row?;
        let snapshot = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({}));
        if let Some(label) = snapshot
            .get("labelSnapshot")
            .and_then(serde_json::Value::as_str)
            .filter(|label| !label.is_empty())
        {
            labels.entry(id).or_default().push(label.to_owned());
        }
    }
    let mut seen = HashSet::new();
    Ok(page_ids
        .iter()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| {
            labels.remove(id).map(|values| NoteSourceLabel {
                page_id: id.clone(),
                label: values.join(" · "),
            })
        })
        .collect())
}

#[tauri::command]
pub fn list_note_source_labels(
    store: State<'_, AppStore>,
    page_ids: Vec<String>,
) -> Result<Vec<NoteSourceLabel>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    source_labels_from(&connection, &page_ids).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_labels_project_only_selected_snapshots_without_other_tables() {
        let connection = Connection::open_in_memory().expect("open fixture");
        connection.execute_batch(
            "create table note_links(source_page_id text, target_kind text, target_snapshot_json text, created_at text);
             insert into note_links values
             ('first','paper','{\"labelSnapshot\":\"Later\"}','02'),
             ('first','manuscript','{\"labelSnapshot\":\"Earlier\"}','01'),
             ('first','note_page','{\"labelSnapshot\":\"Ignored\"}','00'),
             ('second','paper','{\"labelSnapshot\":\"Unselected\"}','00'),
             ('first','paper','{\"labelSnapshot\":\"\"}','03'),
             ('first','paper','{\"labelSnapshot\":42}','04'),
             ('empty','paper','{}','00');",
        ).expect("seed label-only fixture");
        let result = source_labels_from(
            &connection,
            &[
                "missing".into(),
                "first".into(),
                "first".into(),
                "empty".into(),
            ],
        )
        .expect("read without note bodies or target tables");
        assert_eq!(
            serde_json::to_value(result).expect("serialize IPC result"),
            serde_json::json!([{"pageId":"first","label":"Earlier · Later"}])
        );
        assert!(
            source_labels_from(&connection, &["first' OR 1=1 --".into()])
                .expect("bound parameter")
                .is_empty()
        );
    }

    #[test]
    fn source_labels_enforce_the_batch_bound_before_database_access() {
        let connection = Connection::open_in_memory().expect("open empty fixture");
        assert!(
            source_labels_from(&connection, &[])
                .expect("empty query")
                .is_empty()
        );
        assert!(
            matches!(source_labels_from(&connection, &vec!["id".into();501]),
            Err(NotesError::InvalidInput(message)) if message.contains("note_source_labels_limit"))
        );
    }
}
