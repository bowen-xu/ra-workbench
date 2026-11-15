//! Read-side keepalives for assets entering native editor/undo history.
use serde_json::Value;

use crate::{
    hbs_storage,
    storage::{AppStore, StorageError},
};

use super::{
    document_repository::load_document_from, error::NotesError, model::NoteDocumentSnapshot,
};

fn retain_value_assets(store: &AppStore, value: &Value) -> Result<(), StorageError> {
    match value {
        Value::Object(fields) => {
            if let Some(id) = fields.get("assetId").and_then(Value::as_str)
                && id.len() == 64
                && id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                store.retain_content_for_session(id)?;
            }
            for value in fields.values() {
                retain_value_assets(store, value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                retain_value_assets(store, value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn load_document_for_session(
    store: &AppStore,
    page_id: &str,
) -> Result<NoteDocumentSnapshot, NotesError> {
    let _content_read = store.content_read()?;
    let db = store.connection()?;
    let snapshot = load_document_from(&db, page_id)?;
    fn retain_blocks(
        store: &AppStore,
        blocks: &[super::NoteDocumentBlock],
    ) -> Result<(), StorageError> {
        for block in blocks {
            retain_value_assets(store, &block.content)?;
            retain_blocks(store, &block.children)?;
        }
        Ok(())
    }
    retain_blocks(store, &snapshot.document.blocks)?;
    Ok(snapshot)
}

pub(crate) enum HbsRead {
    Document,
    Preservation,
}

/// Keep every returned note snapshot root, including B and S-prime ancestors.
/// This adds resource lifetime ownership, without changing any HBS state.
pub(crate) fn read_hbs_for_session(
    store: &AppStore,
    kind: &str,
    resource_id: &str,
    read: HbsRead,
) -> Result<Value, String> {
    let _content_read = store.content_read().map_err(|error| error.to_string())?;
    let db = store.connection().map_err(|error| error.to_string())?;
    let snapshot = match read {
        HbsRead::Document => hbs_storage::read(&db, kind, resource_id),
        HbsRead::Preservation => hbs_storage::preservation::read(
            store.generation_policy_source(),
            store.paper_reading_source(),
            &db,
            kind,
            resource_id,
        ),
    }?;
    if kind == "note" {
        retain_value_assets(store, &snapshot).map_err(|error| error.to_string())?;
    }
    Ok(snapshot)
}
