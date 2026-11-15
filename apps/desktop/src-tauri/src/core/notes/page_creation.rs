use super::{
    document_repository::load_document_from,
    error::NotesError,
    model::{NoteDocumentSnapshot, NotePageDraft, validate_title},
    page_repository::{
        next_position, position_child_page_block, sync_child_page_block, touch_parents,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub(crate) fn create_page_from(
    connection: &mut Connection,
    draft: NotePageDraft,
) -> Result<NoteDocumentSnapshot, NotesError> {
    create_page_with_paper_from(connection, draft, None)
}

pub(crate) fn create_page_with_paper_from(
    connection: &mut Connection,
    draft: NotePageDraft,
    paper_id: Option<&str>,
) -> Result<NoteDocumentSnapshot, NotesError> {
    let transaction = connection.transaction()?;
    let id = create_page_with_paper_in(&transaction, draft, paper_id, None)?;
    transaction.commit()?;
    load_document_from(connection, &id)
}
pub(crate) fn create_page_with_paper_in(
    transaction: &rusqlite::Transaction<'_>,
    draft: NotePageDraft,
    paper_id: Option<&str>,
    assigned_id: Option<&str>,
) -> Result<String, NotesError> {
    let title = validate_title(&draft.title)
        .ok_or_else(|| NotesError::InvalidInput("笔记标题过长".to_owned()))?;
    if draft.parent_page_id.is_none() && draft.parent_block_position.is_some() {
        return Err(NotesError::InvalidInput(
            "顶层页面不能指定父页面块位置".to_owned(),
        ));
    }
    if draft
        .parent_block_position
        .is_some_and(|position| position < 0)
    {
        return Err(NotesError::InvalidInput("子页面块位置无效".to_owned()));
    }
    if let Some(parent) = &draft.parent_page_id {
        let exists = transaction.query_row(
            "select exists(select 1 from note_pages where id = ?1 and trashed_at is null)",
            [parent],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(NotesError::InvalidInput("父页面不存在".to_owned()));
        }
    }
    let id = assigned_id
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let block_id = Uuid::new_v4().to_string();
    let position = next_position(transaction, draft.parent_page_id.as_deref())?;
    transaction.execute(
        "insert into note_pages
         (id, title, parent_page_id, position, status, is_favorite, revision, schema_version, created_at, updated_at)
         values (?1, ?2, ?3, ?4, 'active', 0, 0, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![id, title, draft.parent_page_id, position],
    )?;
    transaction.execute(
        "insert into note_blocks
         (id, scope_kind, page_id, parent_block_id, position, block_type, props_json,
          content_json, plain_text, schema_version, revision, status, is_favorite, created_at, updated_at)
         values (?1, 'page', ?2, null, 0, 'paragraph', '{}',
                 '{\"kind\":\"inline\",\"items\":[]}', '', 1, 0, 'active', 0,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![block_id, id],
    )?;
    // Assigned identities are used by atomic change groups whose parent document
    // already owns the exact child-page block identity. The caller saves it.
    if draft.parent_page_id.is_some() && assigned_id.is_none() {
        let parents = sync_child_page_block(transaction, &id)?;
        if let (Some(parent), Some(position)) = (&draft.parent_page_id, draft.parent_block_position)
        {
            position_child_page_block(transaction, parent, &id, position)?;
        }
        touch_parents(transaction, parents)?;
    }
    if let Some(paper_id) = paper_id {
        let paper_title: String = transaction
            .query_row(
                "select title from bibliographic_items where id = ?1 and trashed_at is null",
                [paper_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(NotesError::NotFound)?;
        super::link_repository::upsert_link_from(
            transaction,
            super::model::NoteLinkDraft {
                source_page_id: id.clone(),
                source_block_id: None,
                target: super::model::NoteLinkTarget {
                    kind: "paper".to_owned(),
                    id: paper_id.to_owned(),
                    label_snapshot: paper_title,
                    quote_snapshot: None,
                },
                relation: "derived_from".to_owned(),
            },
        )?;
    }
    Ok(id)
}
