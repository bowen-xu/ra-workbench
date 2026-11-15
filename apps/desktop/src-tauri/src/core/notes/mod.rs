#[path = "agent_context.rs"]
pub mod agent_context;
#[path = "agent_rewrite.rs"]
pub(crate) mod agent_rewrite;
#[path = "assets.rs"]
pub(crate) mod assets;
mod export_assets;
pub(crate) use export_assets::{NoteExportAssetReader, NoteExportAssets};
#[path = "catalog/mod.rs"]
mod catalog;
#[cfg(test)]
pub(crate) use error::NotesCommandError;
pub(crate) use error::NotesError;
#[path = "commands.rs"]
mod commands;
#[path = "draft.rs"]
pub(crate) mod draft;
pub(crate) use catalog::document_text as project_catalog_document_text;
pub use catalog::query_note_page_catalog;
pub(crate) use catalog::register_projection_functions;
#[path = "conflict_resolution.rs"]
mod conflict_resolution;
#[cfg(test)]
#[path = "conflict_resolution_tests.rs"]
mod conflict_resolution_tests;
pub use conflict_resolution::{load_note_revision, resolve_note_conflict};
#[path = "document_repository.rs"]
mod document_repository;
#[path = "reading_agent_context.rs"]
pub mod reading_agent_context;
#[path = "text_view/mod.rs"]
pub(crate) mod text_view;
pub(crate) use document_repository::{load_document_value, save_document_in};
pub(crate) use model::NotePageDraft;
pub(crate) use model::validate_title;
pub(crate) use model::{NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest};
pub(crate) use page_creation::create_page_with_paper_in;
#[path = "document_analysis.rs"]
pub(crate) mod document_analysis;
#[path = "document_validation.rs"]
pub(crate) mod document_validation;
#[path = "error.rs"]
mod error;
#[path = "favorites.rs"]
mod favorites;
#[path = "index_documents.rs"]
mod index_documents;
#[path = "lifecycle.rs"]
mod lifecycle;
#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
#[path = "link_repository.rs"]
mod link_repository;
pub use index_documents::load_note_index_documents;
#[path = "source_labels.rs"]
mod source_labels;
pub use source_labels::list_note_source_labels;
#[path = "literature_notes.rs"]
mod literature_notes;
#[path = "navigation.rs"]
mod navigation;
pub use favorites::{list_note_favorites, reorder_note_favorites};
pub use navigation::commit_note_navigation;
#[path = "metadata_batch.rs"]
mod metadata_batch;
#[path = "metadata_entry_history.rs"]
mod metadata_entry_history;
#[path = "metadata_history.rs"]
mod metadata_history;
pub use metadata_history::replay_note_metadata_batch;
#[path = "model.rs"]
mod model;
#[path = "page_entries.rs"]
mod page_entries;
#[path = "page_entry_lifecycle.rs"]
pub(crate) mod page_entry_lifecycle;
pub use metadata_batch::{apply_note_metadata_batch, list_note_page_metadata};
#[cfg(test)]
#[path = "note_index_tests.rs"]
mod note_index_tests;
#[path = "page_creation.rs"]
mod page_creation;
#[path = "page_mutation.rs"]
mod page_mutation;
#[path = "page_order.rs"]
mod page_order;
#[path = "page_repository.rs"]
mod page_repository;
#[cfg(test)]
#[path = "performance_tests.rs"]
mod performance_tests;
#[path = "position_note.rs"]
mod position_note;
#[cfg(test)]
#[path = "reading_link_migration_tests.rs"]
mod reading_link_migration_tests;
#[path = "reading_note_repository.rs"]
mod reading_note_repository;
#[path = "revision_assets.rs"]
pub(crate) mod revision_assets;
#[path = "search.rs"]
mod search;
#[path = "session_assets.rs"]
pub(crate) mod session_assets;
#[path = "tags.rs"]
mod tags;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "text_change_set.rs"]
pub(crate) mod text_change_set;
#[path = "transfer.rs"]
mod transfer;
#[path = "transfer_manager.rs"]
mod transfer_manager;

pub(crate) use assets::NoteAssetManager;
pub use assets::{cancel_note_asset_import, import_note_asset, read_note_asset};
pub use commands::{
    cancel_note_index_rebuild, create_literature_note, create_note_page, create_reading_note,
    duplicate_note_page, get_note_index_state, get_reading_note, list_literature_notes,
    list_note_backlinks, list_note_links, list_note_pages, list_note_revisions, list_reading_notes,
    load_note_document, move_note_page, permanently_delete_note_page,
    permanently_delete_reading_note, rebuild_note_index, remove_note_link, restore_note_revision,
    save_note_document, search_notes, set_note_page_status, set_note_revision_pinned,
    set_reading_note_status, update_note_page, update_reading_note, upsert_note_link,
};
pub(crate) use search::NoteIndexManager;
pub use tags::{ensure_note_tag, list_note_tags};
pub use transfer::cancel_note_export;
pub use transfer::export_notes;
pub(crate) use transfer_manager::NoteTransferManager;

#[cfg(test)]
pub(crate) use tests::insert_reading_fixture;
#[path = "hbs.rs"]
pub(crate) mod hbs;
#[cfg(test)]
#[path = "navigation_tests.rs"]
mod navigation_tests;
