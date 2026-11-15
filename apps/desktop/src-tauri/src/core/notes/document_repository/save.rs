//! Shared note save operation. The owner commits or rolls back the whole unit.
use super::{
    child_page_targets, load_document_from, load_document_value, replace_semantic_links,
    write_blocks,
};
use crate::notes::{
    document_validation::validate_document,
    error::NotesError,
    model::{NoteDocumentSaveRequest, NoteDocumentSnapshot},
    page_repository::sync_child_page_block,
    revision_assets::replace_revision_assets,
};
use rusqlite::{Connection, Transaction, params};
use std::collections::HashSet;
use uuid::Uuid;

pub(crate) fn save_document_from(
    connection: &mut Connection,
    request: NoteDocumentSaveRequest,
) -> Result<NoteDocumentSnapshot, NotesError> {
    let transaction = connection.transaction()?;
    let saved = save_document_in(&transaction, request)?;
    transaction.commit()?;
    Ok(saved)
}

/// All reads, history and dependent writes belong to the caller's transaction.
/// The returned snapshot is provisional until that transaction commits. On any
/// error the owner must roll back the entire transaction, including other work.
pub(crate) fn save_document_in(
    transaction: &Transaction<'_>,
    request: NoteDocumentSaveRequest,
) -> Result<NoteDocumentSnapshot, NotesError> {
    validate_document(&request.document).map_err(NotesError::InvalidInput)?;
    save_checked_in(transaction, request)
}
pub(crate) fn save_preserving_in(
    transaction: &Transaction<'_>,
    request: NoteDocumentSaveRequest,
    before: &crate::notes::NoteDocument,
) -> Result<NoteDocumentSnapshot, NotesError> {
    crate::notes::document_validation::validate_preserving(&request.document, before)
        .map_err(NotesError::InvalidInput)?;
    save_checked_in(transaction, request)
}
fn save_checked_in(
    transaction: &Transaction<'_>,
    request: NoteDocumentSaveRequest,
) -> Result<NoteDocumentSnapshot, NotesError> {
    if !matches!(
        request.save_reason.as_str(),
        "autosave" | "manual" | "import" | "restore" | "structure"
    ) {
        return Err(NotesError::InvalidInput("保存原因无效".to_owned()));
    }
    let current_document = load_document_value(transaction, &request.document.page_id)?;
    let requested_child_pages = child_page_targets(&request.document.blocks)?;
    let requested_child_page_set: HashSet<_> = requested_child_pages.iter().cloned().collect();
    let current_revision: i64 = transaction.query_row(
        "select revision from note_pages where id = ?1",
        [&request.document.page_id],
        |row| row.get(0),
    )?;
    if current_revision != request.expected_revision {
        return Err(NotesError::RevisionConflict { current_revision });
    }
    super::super::page_entry_lifecycle::reconcile(
        transaction,
        &current_document,
        &request.document,
    )?;
    let direct_children = {
        let mut statement = transaction
            .prepare("select id from note_pages where parent_page_id = ?1 and status = 'active'")?;
        statement
            .query_map([&request.document.page_id], |row| row.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?
    };
    if requested_child_page_set
        .iter()
        .any(|target| !direct_children.contains(target))
    {
        return Err(NotesError::InvalidInput(
            "子页面块必须唯一指向当前页面的活动子页面".to_owned(),
        ));
    }
    for (position, child_id) in requested_child_pages.iter().enumerate() {
        transaction.execute(
            "update note_pages set position = ?2,
             revision = revision + case when position <> ?2 then 1 else 0 end,
             updated_at = case when position <> ?2 then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else updated_at end
             where id = ?1",
            params![child_id, position as i64],
        )?;
    }
    let has_revision: bool = transaction.query_row(
        "select exists(select 1 from note_revisions where page_id = ?1)",
        [&request.document.page_id],
        |row| row.get(0),
    )?;
    if request.save_reason != "autosave" || current_revision % 10 == 0 || !has_revision {
        let revision_id = Uuid::new_v4().to_string();
        let inserted = transaction.execute(
            "insert or ignore into note_revisions
             (id, page_id, page_revision, schema_version, document_json, reason, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                revision_id,
                request.document.page_id,
                current_revision,
                current_document.schema_version,
                serde_json::to_string(&current_document)
                    .map_err(|error| NotesError::InvalidInput(error.to_string()))?,
                request.save_reason
            ],
        )?;
        let stored_revision_id = if inserted == 1 {
            revision_id
        } else {
            transaction.query_row(
                "select id from note_revisions where page_id = ?1 and page_revision = ?2",
                params![request.document.page_id, current_revision],
                |row| row.get(0),
            )?
        };
        replace_revision_assets(transaction, &stored_revision_id, &current_document)?;
    }
    let mut retained = HashSet::new();
    write_blocks(
        transaction,
        &request.document.page_id,
        &request.document.blocks,
        None,
        &mut retained,
    )?;
    // Remove obsolete derived links while their source block IDs still exist.
    // ON DELETE SET NULL would otherwise collapse different block references
    // into the same page-level identity before reconciliation can remove them.
    replace_semantic_links(
        transaction,
        &request.document.page_id,
        &request.document.blocks,
    )?;
    let existing = {
        let mut statement =
            transaction.prepare("select id, block_type from note_blocks where page_id = ?1")?;
        statement
            .query_map([&request.document.page_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (id, block_type) in existing {
        if !retained.contains(&id) {
            if block_type == "childPage" {
                transaction.execute(
                    "update note_blocks set status = 'trashed',
                     trashed_at = coalesce(trashed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1 and page_id = ?2",
                    params![id, request.document.page_id],
                )?;
            } else {
                transaction.execute(
                    "delete from note_blocks where id = ?1 and page_id = ?2",
                    params![id, request.document.page_id],
                )?;
            }
        }
    }
    if let Some(title) = &request.document.title {
        let label = crate::notes::model::validate_title(title)
            .ok_or_else(|| NotesError::InvalidInput("笔记标题过长".into()))?;
        transaction.execute(
            "update note_pages set title = ?2, document_title = ?3 where id = ?1",
            params![request.document.page_id, label, title],
        )?;
        if current_document.title.as_ref() != Some(title) {
            let parents = sync_child_page_block(transaction, &request.document.page_id)?;
            crate::notes::page_repository::touch_parents(transaction, parents)?;
        }
    }
    transaction.execute(
        "update note_pages set revision = revision + 1, schema_version = ?2,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
        params![request.document.page_id, request.document.schema_version],
    )?;
    transaction.execute(
        "update note_index_state set state = 'dirty' where id = 1",
        [],
    )?;
    load_document_from(transaction, &request.document.page_id)
}

#[cfg(test)]
#[path = "save/tests.rs"]
mod tests;
