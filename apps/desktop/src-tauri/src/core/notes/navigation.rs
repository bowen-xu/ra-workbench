//! One sidebar gesture commits both link placement and favorites, or neither.
use super::{
    document_repository::{load_document_from, save_preserving_in},
    error::{NotesCommandError, NotesError},
    favorites,
    model::{NoteDocumentSaveRequest, NoteDocumentSnapshot, NotePageMetadataUpdate},
    page_repository::update_page_in,
};
use crate::storage::AppStore;
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::collections::HashSet;
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavigationCommit {
    pub documents: Vec<NoteDocumentSaveRequest>,
    pub expected_favorite_ids: Vec<String>,
    pub favorite_ids: Vec<String>,
}
pub(crate) fn commit_from(
    db: &mut Connection,
    request: NavigationCommit,
) -> Result<Vec<NoteDocumentSnapshot>, NotesError> {
    let tx = db.transaction()?;
    let previous = favorites::list_from(&tx)?;
    let desired: HashSet<_> = request.favorite_ids.iter().cloned().collect();
    let ids: HashSet<_> = request
        .documents
        .iter()
        .map(|save| save.document.page_id.clone())
        .collect();
    if previous != request.expected_favorite_ids
        || desired.len() != request.favorite_ids.len()
        || request.documents.len() > 2
        || ids.len() != request.documents.len()
    {
        return Err(NotesError::InvalidInput(
            "笔记导航已变化，请刷新后重试。".into(),
        ));
    }
    let mut changed = Vec::new();
    for save in request.documents {
        let before = load_document_from(&tx, &save.document.page_id)?;
        let review_before = super::text_change_set::read(&tx, &before.page.id)?;
        crate::agent::text_tools::note_review::check_navigation(&tx, &review_before)
            .map_err(NotesError::InvalidInput)?;
        if before.page.status != "active" {
            return Err(NotesError::InvalidInput(
                "请先恢复笔记，再移动页面链接。".into(),
            ));
        }
        changed.push(before.page.id.clone());
        save_preserving_in(&tx, save, &before.document)?;
        let review_after = super::text_change_set::read(&tx, &before.page.id)?;
        crate::agent::text_tools::note_review::synchronize_navigation(
            &tx,
            &review_before,
            &review_after,
        )
        .map_err(NotesError::InvalidInput)?;
    }
    let previous_set: HashSet<_> = previous.iter().cloned().collect();
    for id in previous_set.symmetric_difference(&desired) {
        let before = load_document_from(&tx, id)?;
        if desired.contains(id) && before.page.status != "active" {
            return Err(NotesError::InvalidInput(
                "只能将活动独立笔记加入收藏。".into(),
            ));
        }
        update_page_in(
            &tx,
            NotePageMetadataUpdate {
                page_id: id.clone(),
                title: None,
                is_favorite: Some(desired.contains(id)),
                project_ids: None,
                tag_ids: None,
                expected_revision: before.page.revision,
            },
        )?;
        if !changed.contains(id) {
            changed.push(id.clone());
        }
    }
    for (position, id) in request.favorite_ids.iter().enumerate() {
        tx.execute(
            "update note_favorite_order set position=?2 where page_id=?1",
            params![id, position as i64],
        )?;
    }
    let result = changed
        .iter()
        .map(|id| load_document_from(&tx, id))
        .collect::<Result<Vec<_>, _>>()?;
    tx.commit()?;
    Ok(result)
}
#[tauri::command]
pub fn commit_note_navigation(
    store: State<'_, AppStore>,
    request: NavigationCommit,
) -> Result<Vec<NoteDocumentSnapshot>, NotesCommandError> {
    let mut db = store.connection().map_err(NotesError::from)?;
    commit_from(&mut db, request).map_err(Into::into)
}
