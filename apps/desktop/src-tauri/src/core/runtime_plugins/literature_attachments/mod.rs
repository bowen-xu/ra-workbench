//! Fixed attachment operations; Core retains all file, SQL and lifetime owners.
pub(crate) mod commands;
mod execute;
mod failure;
mod resources;

pub(crate) use execute::attach;
pub(crate) use execute::content_path;
pub(crate) use execute::execute;
pub(crate) use failure::{AttachmentFailure, AttachmentProgress, AttachmentSql};
use research_workbench_native_sdk::{
    attachments::{AttachmentRemovalResult, PageText, ReadingPosition, SaveReadingStateInputV1},
    catalog::CatalogAttachment,
};
pub(crate) use resources::AttachmentResources;

pub(crate) enum AttachmentRequest {
    Attach {
        item_id: String,
        path: String,
    },
    Rename {
        item_id: String,
        attachment_id: String,
        display_name: String,
    },
    Remove {
        item_id: String,
        attachment_id: String,
    },
    Read {
        attachment_id: String,
    },
    Reveal {
        attachment_ids: Vec<String>,
        app: tauri::AppHandle,
    },
    SaveState {
        input: SaveReadingStateInputV1,
    },
    GetState {
        item_id: String,
        attachment_id: String,
    },
    IsIndexed {
        attachment_id: String,
    },
    Index {
        item_id: String,
        attachment_id: String,
        pages: Vec<PageText>,
    },
}

#[derive(Debug)]
pub(crate) enum AttachmentResult {
    Attachment(CatalogAttachment),
    Removed(AttachmentRemovalResult),
    Bytes(Vec<u8>),
    State(Option<ReadingPosition>),
    Indexed(bool),
    Done,
}

#[cfg(test)]
mod tests;

pub(crate) use execute::attach_source;
