use super::{
    error::NotesError,
    model::{NotePage, NotePageMoveRequest, NoteStatusUpdate, validate_status},
    page_order::{apply_sibling_order, sibling_ids},
    page_repository::{page_from, sync_child_page_block, touch_parents},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashSet;
pub(crate) fn move_page_from(
    connection: &mut Connection,
    request: NotePageMoveRequest,
) -> Result<NotePage, NotesError> {
    let transaction = connection.transaction()?;
    let result = move_page_in(&transaction, request)?;
    transaction.commit()?;
    Ok(result)
}
pub(crate) fn move_page_in(
    transaction: &Transaction<'_>,
    request: NotePageMoveRequest,
) -> Result<NotePage, NotesError> {
    if request.position < 0 || request.parent_page_id.as_deref() == Some(&request.page_id) {
        return Err(NotesError::InvalidInput("页面位置无效".to_owned()));
    }
    let (current_revision, current_parent): (i64, Option<String>) = transaction
        .query_row(
            "select revision, parent_page_id from note_pages where id = ?1",
            [&request.page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    if current_revision != request.expected_revision {
        return Err(NotesError::RevisionConflict { current_revision });
    }
    if let Some(parent) = &request.parent_page_id {
        let exists = transaction.query_row(
            "select exists(select 1 from note_pages where id = ?1 and status <> 'trashed')",
            [parent],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(NotesError::InvalidInput("父页面不存在".to_owned()));
        }
    }
    let mut cursor = request.parent_page_id.clone();
    while let Some(parent) = cursor {
        if parent == request.page_id {
            return Err(NotesError::InvalidInput("页面树不能形成环路".to_owned()));
        }
        cursor = transaction
            .query_row(
                "select parent_page_id from note_pages where id = ?1",
                [&parent],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
    }
    if current_parent == request.parent_page_id {
        let mut ordered = sibling_ids(transaction, current_parent.as_deref(), &request.page_id)?;
        let position = usize::try_from(request.position)
            .unwrap_or(usize::MAX)
            .min(ordered.len());
        ordered.insert(position, request.page_id.clone());
        apply_sibling_order(
            transaction,
            request.parent_page_id.as_deref(),
            &ordered,
            &request.page_id,
        )?;
    } else {
        let old_order = sibling_ids(transaction, current_parent.as_deref(), &request.page_id)?;
        apply_sibling_order(
            transaction,
            current_parent.as_deref(),
            &old_order,
            &request.page_id,
        )?;
        let mut new_order = sibling_ids(
            transaction,
            request.parent_page_id.as_deref(),
            &request.page_id,
        )?;
        let position = usize::try_from(request.position)
            .unwrap_or(usize::MAX)
            .min(new_order.len());
        new_order.insert(position, request.page_id.clone());
        apply_sibling_order(
            transaction,
            request.parent_page_id.as_deref(),
            &new_order,
            &request.page_id,
        )?;
    }
    let parents = sync_child_page_block(transaction, &request.page_id)?;
    touch_parents(transaction, parents)?;
    transaction.execute(
        "update note_index_state set state = 'dirty' where id = 1",
        [],
    )?;
    page_from(transaction, &request.page_id)
}

pub(crate) fn set_page_status_from(
    connection: &mut Connection,
    request: NoteStatusUpdate,
) -> Result<NotePage, NotesError> {
    let transaction = connection.transaction()?;
    let result = set_page_status_in(&transaction, request)?;
    transaction.commit()?;
    Ok(result)
}
pub(crate) fn set_page_status_in(
    transaction: &Transaction<'_>,
    request: NoteStatusUpdate,
) -> Result<NotePage, NotesError> {
    if !validate_status(&request.status) {
        return Err(NotesError::InvalidInput("笔记状态无效".to_owned()));
    }
    let current_revision: i64 = transaction
        .query_row(
            "select revision from note_pages where id = ?1",
            [&request.id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    if current_revision != request.expected_revision {
        return Err(NotesError::RevisionConflict { current_revision });
    }
    let ids = if matches!(request.status.as_str(), "active" | "trashed") {
        let mut statement = transaction.prepare(
            "with recursive subtree(id) as (
               select ?1 union all select p.id from note_pages p join subtree s on p.parent_page_id = s.id
             ) select id from subtree",
        )?;
        statement
            .query_map([&request.id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![request.id.clone()]
    };
    for id in &ids {
        transaction.execute("update note_pages set status = ?2, trashed_at = case when ?2 = 'trashed' then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else null end, revision = revision + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1", params![id, request.status])?;
    }
    let mut parents = HashSet::new();
    for id in &ids {
        parents.extend(sync_child_page_block(transaction, id)?);
    }
    touch_parents(transaction, parents)?;
    page_from(transaction, &request.id)
}
