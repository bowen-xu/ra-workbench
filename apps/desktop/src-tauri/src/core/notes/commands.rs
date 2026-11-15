use tauri::State;
use uuid::Uuid;

use crate::storage::AppStore;

use super::{
    document_repository::{
        list_revisions_from, load_document_from, save_document_from, set_revision_pinned_from,
    },
    error::{NotesCommandError, NotesError},
    lifecycle::{permanently_delete_page_from, permanently_delete_reading_note_from},
    link_repository::{
        list_backlinks_from, list_outgoing_from, remove_link_from, upsert_link_from,
    },
    model::{
        NoteDocumentBlock, NoteDocumentSaveRequest, NoteDocumentSnapshot, NoteLink, NoteLinkDraft,
        NoteListQuery, NotePage, NotePageDraft, NotePageMetadataUpdate, NotePageMoveRequest,
        NotePermanentDeleteReport, NotePermanentDeleteRequest, NoteRevisionSummary, NoteSearchPage,
        NoteSearchQuery, NoteStatusUpdate, ReadingNoteBlock, ReadingNoteDraft,
        ReadingNoteListQuery, ReadingNoteUpdate,
    },
    page_repository::{
        create_page_from, list_pages_from, move_page_from, set_page_status_from, update_page_from,
    },
    reading_note_repository::{
        create_reading_note_from, list_reading_notes_from, reading_note_from,
        set_reading_note_status_from, update_reading_note_from,
    },
    search::{NoteIndexManager, index_state_from, rebuild_note_index_from, search_notes_from},
};

#[tauri::command]
pub fn list_note_pages(
    store: State<'_, AppStore>,
    query: Option<NoteListQuery>,
) -> Result<Vec<NotePage>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    list_pages_from(&connection, query.as_ref()).map_err(Into::into)
}

#[tauri::command]
pub fn create_note_page(
    store: State<'_, AppStore>,
    draft: NotePageDraft,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    create_page_from(&mut connection, draft).map_err(Into::into)
}

#[tauri::command]
pub fn update_note_page(
    store: State<'_, AppStore>,
    request: NotePageMetadataUpdate,
) -> Result<NotePage, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    update_page_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn move_note_page(
    store: State<'_, AppStore>,
    request: NotePageMoveRequest,
) -> Result<NotePage, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    move_page_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn set_note_page_status(
    store: State<'_, AppStore>,
    request: NoteStatusUpdate,
) -> Result<NotePage, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    set_page_status_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn permanently_delete_note_page(
    store: State<'_, AppStore>,
    request: NotePermanentDeleteRequest,
) -> Result<NotePermanentDeleteReport, NotesCommandError> {
    let content_directory = store.content_directory().to_path_buf();
    let mut connection = store.connection().map_err(NotesError::from)?;
    permanently_delete_page_from(&mut connection, &content_directory, request).map_err(Into::into)
}

#[tauri::command]
pub async fn load_note_document(
    store: State<'_, AppStore>,
    page_id: String,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    store
        .blocking(move |store| {
            super::session_assets::load_document_for_session(store, &page_id).map_err(Into::into)
        })
        .await
}

#[tauri::command]
pub fn save_note_document(
    store: State<'_, AppStore>,
    request: NoteDocumentSaveRequest,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    save_document_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn list_note_revisions(
    store: State<'_, AppStore>,
    page_id: String,
) -> Result<Vec<NoteRevisionSummary>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    list_revisions_from(&connection, &page_id).map_err(Into::into)
}

#[tauri::command]
pub fn set_note_revision_pinned(
    store: State<'_, AppStore>,
    page_id: String,
    revision_id: String,
    is_pinned: bool,
) -> Result<NoteRevisionSummary, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    set_revision_pinned_from(&connection, &page_id, &revision_id, is_pinned).map_err(Into::into)
}

#[tauri::command]
pub fn restore_note_revision(
    store: State<'_, AppStore>,
    page_id: String,
    revision_id: String,
    expected_revision: i64,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    let tx = connection.transaction().map_err(NotesError::from)?;
    let mut document = super::conflict_resolution::revision(&tx, &page_id, &revision_id)?;
    document.page_id = page_id.clone();
    let saved = super::document_repository::save_document_in(
        &tx,
        NoteDocumentSaveRequest {
            document,
            expected_revision,
            save_reason: "restore".into(),
        },
    )?;
    super::conflict_resolution::retire_reviews(&tx, &page_id)?;
    tx.commit().map_err(NotesError::from)?;
    Ok(saved)
}

fn renew_block_ids(blocks: &mut [NoteDocumentBlock]) {
    for block in blocks {
        block.id = Uuid::new_v4().to_string();
        renew_block_ids(&mut block.children);
    }
}

#[tauri::command]
pub fn duplicate_note_page(
    store: State<'_, AppStore>,
    page_id: String,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    let source = load_document_from(&connection, &page_id)?;
    let created = create_page_from(
        &mut connection,
        NotePageDraft {
            title: format!("{} 副本", source.page.title),
            parent_page_id: source.page.parent_page_id,
            parent_block_position: None,
        },
    )?;
    let mut document = source.document;
    document.page_id = created.page.id.clone();
    document.title = Some(created.page.title.clone());
    renew_block_ids(&mut document.blocks);
    save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document,
            expected_revision: created.page.revision,
            save_reason: "structure".to_owned(),
        },
    )
    .map_err(Into::into)
}

#[tauri::command]
pub async fn list_reading_notes(
    store: State<'_, AppStore>,
    query: Option<ReadingNoteListQuery>,
) -> Result<Vec<ReadingNoteBlock>, NotesCommandError> {
    store
        .query(move |db| list_reading_notes_from(db, query.as_ref()).map_err(Into::into))
        .await
}

#[tauri::command]
pub async fn get_reading_note(
    store: State<'_, AppStore>,
    id: String,
) -> Result<ReadingNoteBlock, NotesCommandError> {
    store
        .query(move |db| reading_note_from(db, &id).map_err(Into::into))
        .await
}

#[tauri::command]
pub fn create_reading_note(
    store: State<'_, AppStore>,
    draft: ReadingNoteDraft,
) -> Result<ReadingNoteBlock, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    create_reading_note_from(&mut connection, draft).map_err(Into::into)
}

#[tauri::command]
pub fn update_reading_note(
    store: State<'_, AppStore>,
    request: ReadingNoteUpdate,
) -> Result<ReadingNoteBlock, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    update_reading_note_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn set_reading_note_status(
    store: State<'_, AppStore>,
    request: NoteStatusUpdate,
) -> Result<ReadingNoteBlock, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    set_reading_note_status_from(&mut connection, request).map_err(Into::into)
}

#[tauri::command]
pub fn permanently_delete_reading_note(
    store: State<'_, AppStore>,
    request: NotePermanentDeleteRequest,
) -> Result<NotePermanentDeleteReport, NotesCommandError> {
    let content_directory = store.content_directory().to_path_buf();
    let mut connection = store.connection().map_err(NotesError::from)?;
    permanently_delete_reading_note_from(&mut connection, &content_directory, request)
        .map_err(Into::into)
}

#[tauri::command]
pub fn upsert_note_link(
    store: State<'_, AppStore>,
    draft: NoteLinkDraft,
) -> Result<NoteLink, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    upsert_link_from(&connection, draft).map_err(Into::into)
}

#[tauri::command]
pub fn remove_note_link(store: State<'_, AppStore>, id: String) -> Result<(), NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    remove_link_from(&connection, &id).map_err(Into::into)
}

#[tauri::command]
pub fn list_note_links(
    store: State<'_, AppStore>,
    page_id: String,
) -> Result<Vec<NoteLink>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    list_outgoing_from(&connection, &page_id).map_err(Into::into)
}

#[tauri::command]
pub fn list_note_backlinks(
    store: State<'_, AppStore>,
    target_kind: String,
    target_id: String,
) -> Result<Vec<NoteLink>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    list_backlinks_from(&connection, &target_kind, &target_id).map_err(Into::into)
}

#[tauri::command]
pub async fn rebuild_note_index(
    store: State<'_, AppStore>,
    manager: State<'_, NoteIndexManager>,
) -> Result<(), NotesCommandError> {
    let store = store.inner().clone();
    let manager = manager.inner().clone();
    let interrupt = {
        let connection = store.connection().map_err(NotesError::from)?;
        connection.get_interrupt_handle()
    };
    manager.begin(interrupt)?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut connection = store.connection().map_err(NotesError::from)?;
        rebuild_note_index_from(&mut connection)
    })
    .await;
    manager.finish();
    result
        .map_err(|error| NotesError::IndexFailed(error.to_string()))?
        .map_err(Into::into)
}

#[tauri::command]
pub fn cancel_note_index_rebuild(
    manager: State<'_, NoteIndexManager>,
) -> Result<bool, NotesCommandError> {
    manager.cancel().map_err(Into::into)
}

#[tauri::command]
pub fn get_note_index_state(store: State<'_, AppStore>) -> Result<String, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    index_state_from(&connection).map_err(Into::into)
}

#[tauri::command]
pub async fn search_notes(
    store: State<'_, AppStore>,
    query: NoteSearchQuery,
) -> Result<NoteSearchPage, NotesCommandError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection().map_err(NotesError::from)?;
        search_notes_from(&connection, query)
    })
    .await
    .map_err(|error| NotesError::IndexFailed(error.to_string()))?
    .map_err(Into::into)
}

#[tauri::command]
pub fn create_literature_note(
    store: State<'_, AppStore>,
    paper_id: String,
) -> Result<NoteDocumentSnapshot, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    super::page_repository::create_page_with_paper_from(
        &mut connection,
        NotePageDraft {
            title: "文献笔记".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some(&paper_id),
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn list_literature_notes(
    store: State<'_, AppStore>,
    paper_id: String,
) -> Result<Vec<super::literature_notes::LiteratureNoteSummary>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    super::literature_notes::list_from(&connection, &paper_id).map_err(Into::into)
}
