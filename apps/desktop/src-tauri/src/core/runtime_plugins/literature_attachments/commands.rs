//! Fixed public commands; no arbitrary invoke, SQL or stored-path endpoint.
use super::{AttachmentRequest, AttachmentResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    attachments::{AttachmentRemovalResult, PageText, ReadingPosition, SaveReadingStateInputV1},
    catalog::CatalogAttachment,
    duplicate_review::F64BitsV1,
};
use std::sync::Arc;
use tauri::{State, ipc::Response};

type Service<'a> = State<'a, Arc<LiteratureTraceService>>;
type Store<'a> = State<'a, AppStore>;

#[tauri::command]
pub(crate) async fn attach_runtime_pdf_path(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    path: String,
) -> NativeResult<CatalogAttachment> {
    match service
        .attachments(
            store.inner().clone(),
            package_key,
            AttachmentRequest::Attach { item_id, path },
        )
        .await?
    {
        AttachmentResult::Attachment(value) => Ok(value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn rename_runtime_literature_attachment(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    attachment_id: String,
    display_name: String,
) -> NativeResult<()> {
    done(
        service
            .attachments(
                store.inner().clone(),
                package_key,
                AttachmentRequest::Rename {
                    item_id,
                    attachment_id,
                    display_name,
                },
            )
            .await?,
    )
}
#[tauri::command]
pub(crate) async fn remove_runtime_literature_attachment(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    attachment_id: String,
) -> NativeResult<AttachmentRemovalResult> {
    match service
        .attachments(
            store.inner().clone(),
            package_key,
            AttachmentRequest::Remove {
                item_id,
                attachment_id,
            },
        )
        .await?
    {
        AttachmentResult::Removed(value) => Ok(value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn read_runtime_attachment(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    attachment_id: String,
) -> NativeResult<Response> {
    match service
        .attachments(
            store.inner().clone(),
            package_key,
            AttachmentRequest::Read { attachment_id },
        )
        .await?
    {
        AttachmentResult::Bytes(bytes) => Ok(Response::new(bytes)),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn reveal_runtime_attachments_in_file_manager(
    app: tauri::AppHandle,
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    attachment_ids: Vec<String>,
) -> NativeResult<()> {
    done(
        service
            .attachments(
                store.inner().clone(),
                package_key,
                AttachmentRequest::Reveal {
                    attachment_ids,
                    app,
                },
            )
            .await?,
    )
}
#[allow(clippy::too_many_arguments)] // Preserve the original fixed command fields.
#[tauri::command]
pub(crate) async fn save_runtime_reading_state(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    attachment_id: String,
    page_index: i32,
    zoom: f64,
    view_state_json: Option<String>,
) -> NativeResult<()> {
    done(
        service
            .attachments(
                store.inner().clone(),
                package_key,
                AttachmentRequest::SaveState {
                    input: SaveReadingStateInputV1 {
                        item_id,
                        attachment_id,
                        page_index,
                        zoom: F64BitsV1::from_f64(zoom),
                        view_state_json,
                    },
                },
            )
            .await?,
    )
}
#[tauri::command]
pub(crate) async fn get_runtime_reading_state(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    attachment_id: String,
) -> NativeResult<Option<ReadingPosition>> {
    match service
        .attachments(
            store.inner().clone(),
            package_key,
            AttachmentRequest::GetState {
                item_id,
                attachment_id,
            },
        )
        .await?
    {
        AttachmentResult::State(value) => Ok(value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn runtime_attachment_is_indexed(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    attachment_id: String,
) -> NativeResult<bool> {
    match service
        .attachments(
            store.inner().clone(),
            package_key,
            AttachmentRequest::IsIndexed { attachment_id },
        )
        .await?
    {
        AttachmentResult::Indexed(value) => Ok(value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn index_runtime_attachment_text(
    store: Store<'_>,
    service: Service<'_>,
    package_key: String,
    item_id: String,
    attachment_id: String,
    pages: Vec<PageText>,
) -> NativeResult<()> {
    done(
        service
            .attachments(
                store.inner().clone(),
                package_key,
                AttachmentRequest::Index {
                    item_id,
                    attachment_id,
                    pages,
                },
            )
            .await?,
    )
}
fn done(result: AttachmentResult) -> NativeResult<()> {
    match result {
        AttachmentResult::Done => Ok(()),
        _ => Err(mismatch()),
    }
}
fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "附件结果与本次调用不匹配。",
    )
}
