use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::storage::AppStore;

use super::{
    document_repository::load_document_value,
    error::{NotesCommandError, NotesError},
    model::NoteDocument,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteIndexDocument {
    pub page_id: String,
    pub revision: i64,
    pub document: NoteDocument,
}

/// Keep decoding/flattening in the existing frontend domain function. This
/// projection only batches authoritative revision/document reads, never saves.
pub(crate) fn index_documents_from(
    connection: &Connection,
    page_ids: &[String],
) -> Result<Vec<NoteIndexDocument>, NotesError> {
    if page_ids.len() > 20 {
        return Err(NotesError::InvalidInput(
            "notes_index_documents_limit：一次最多读取 20 篇笔记的索引正文。".to_owned(),
        ));
    }
    if page_ids.is_empty() {
        return Ok(Vec::new());
    }
    let transaction = connection.unchecked_transaction()?;
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for id in page_ids.iter().filter(|id| seen.insert(*id)) {
        let revision = transaction
            .query_row("select revision from note_pages where id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(NotesError::NotFound)?;
        output.push(NoteIndexDocument {
            page_id: id.clone(),
            revision,
            document: load_document_value(&transaction, id)?,
        });
    }
    transaction.commit()?;
    Ok(output)
}

#[tauri::command]
pub fn load_note_index_documents(
    store: State<'_, AppStore>,
    page_ids: Vec<String>,
) -> Result<Vec<NoteIndexDocument>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    index_documents_from(&connection, &page_ids).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_documents_reuse_the_document_loader_without_related_tables_or_writes() {
        let connection = Connection::open_in_memory().expect("open fixture");
        connection.execute_batch(
            "create table note_pages(id text primary key, revision integer, schema_version integer, title text, document_title text);
             create table note_blocks(id text, page_id text, scope_kind text, status text, parent_block_id text, position integer, block_type text, props_json text, content_json text);
             insert into note_pages values('first',7,1,'First',null),('second',8,1,'Second',null);
             insert into note_blocks values
              ('child','first','page','active','parent',0,'formula','{}','{\"kind\":\"formula\",\"source\":\"E=mc^2\"}'),
              ('parent','first','page','active',null,0,'paragraph','{}','{\"kind\":\"inline\",\"items\":[{\"type\":\"text\",\"text\":\"Synthetic\",\"styles\":[]}]}'),
              ('other','second','page','active',null,0,'paragraph','{}','{\"kind\":\"inline\",\"items\":[]}');",
        ).expect("seed document-only fixture");
        let before = connection.total_changes();
        let result = index_documents_from(&connection, &["first".into(), "first".into()])
            .expect("load without links, history or target tables");
        assert_eq!(result.len(), 1);
        let json = serde_json::to_value(&result[0]).expect("serialize IPC projection");
        assert_eq!(json["pageId"], "first");
        assert_eq!(json["revision"], 7);
        assert_eq!(
            json["document"],
            serde_json::to_value(
                load_document_value(&connection, "first").expect("existing loader")
            )
            .expect("serialize existing document")
        );
        assert_eq!(
            result[0].document.blocks[0].children[0].content["source"],
            "E=mc^2"
        );
        assert_eq!(connection.total_changes(), before);
        assert!(matches!(
            index_documents_from(&connection, &["first".into(), "missing".into()]),
            Err(NotesError::NotFound)
        ));
        assert!(connection.is_autocommit());
        assert_eq!(connection.total_changes(), before);
    }

    #[test]
    fn index_document_limits_apply_before_any_database_query() {
        let connection = Connection::open_in_memory().expect("open fixture");
        assert!(
            index_documents_from(&connection, &[])
                .expect("empty query")
                .is_empty()
        );
        assert!(
            matches!(index_documents_from(&connection, &vec!["id".into();21]),
            Err(NotesError::InvalidInput(message)) if message.contains("notes_index_documents_limit"))
        );
    }
}
