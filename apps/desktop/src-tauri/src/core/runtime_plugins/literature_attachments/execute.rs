//! Original fixed SQL/IO order. No RPC occurs while these owners are held.
use super::{
    AttachmentFailure, AttachmentProgress, AttachmentRequest, AttachmentResources,
    AttachmentResult, AttachmentSql,
};
use crate::{content::StoredObjectRead, storage::AppStore};
use research_workbench_native_sdk::{
    attachments::{AttachmentPlanV1, AttachmentRemovalResult, ReadingPosition},
    catalog::CatalogAttachment,
    data::DataValue,
    duplicate_review::F64BitsV1,
};
use rusqlite::Connection;
use tauri_plugin_opener::OpenerExt;

mod remove;
mod transaction;

type Result<T> = std::result::Result<T, AttachmentFailure>;
pub(super) fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}
pub(super) fn optional(value: Option<&str>) -> DataValue {
    value.map_or(DataValue::Null, text)
}

/// Internal Core location for Docling, before its existing service wait.
pub(crate) fn content_path(
    db: &Connection,
    content_directory: &std::path::Path,
    resources: &AttachmentResources,
    attachment_id: &str,
) -> Result<std::path::PathBuf> {
    let progress = AttachmentProgress::new("attachment-path");
    let relative = relative_path(db, resources, attachment_id, &progress)?;
    StoredObjectRead::resolve(content_directory, &relative)
        .map(StoredObjectRead::into_legacy_path)
        .map_err(|error| progress.stored(error))
}

pub(crate) fn execute(
    store: &AppStore,
    resources: &AttachmentResources,
    request: AttachmentRequest,
    plan: Option<AttachmentPlanV1>,
) -> Result<AttachmentResult> {
    let progress = AttachmentProgress::new("owner");
    match request {
        AttachmentRequest::Attach { item_id, path } => {
            let _content = store.content_read().map_err(|e| progress.owner(e))?;
            let db = store.connection().map_err(|e| progress.owner(e))?;
            attach(&db, store, resources, &item_id, &path).map(AttachmentResult::Attachment)
        }
        AttachmentRequest::Read { attachment_id } => {
            let _content = store.content_read().map_err(|e| progress.owner(e))?;
            let db = store.connection().map_err(|e| progress.owner(e))?;
            let relative = relative_path(&db, resources, &attachment_id, &progress)?;
            let read = StoredObjectRead::resolve(store.content_directory(), &relative)
                .map_err(|e| progress.stored(e))?;
            drop(db);
            read.read()
                .map(AttachmentResult::Bytes)
                .map_err(|e| progress.io(e))
        }
        AttachmentRequest::Remove {
            item_id,
            attachment_id,
        } => remove::remove(store, resources, &item_id, &attachment_id)
            .map(AttachmentResult::Removed),
        AttachmentRequest::Reveal {
            attachment_ids,
            app,
        } => {
            if attachment_ids.is_empty() {
                return Err(progress.missing());
            }
            let _content = store.content_read().map_err(|e| progress.owner(e))?;
            let db = store.connection().map_err(|e| progress.owner(e))?;
            let paths = attachment_ids
                .iter()
                .map(|id| {
                    let relative = relative_path(&db, resources, id, &progress)?;
                    StoredObjectRead::resolve(store.content_directory(), &relative)
                        .map(StoredObjectRead::into_legacy_path)
                        .map_err(|e| progress.stored(e))
                })
                .collect::<Result<Vec<_>>>()?;
            app.opener().reveal_items_in_dir(paths).map_err(|e| {
                progress.fail(
                    "file_manager_error",
                    format!("无法在系统文件管理器中显示附件：{e}"),
                )
            })?;
            Ok(AttachmentResult::Done)
        }
        AttachmentRequest::Rename {
            item_id,
            attachment_id,
            ..
        } => {
            let db = store.connection().map_err(|e| progress.owner(e))?;
            let Some(AttachmentPlanV1::Rename { display_name, .. }) = plan else {
                return Err(progress.fail("plugin_host_response_mismatch", "附件改名准备缺失。"));
            };
            let mut progress = progress;
            progress.phase = "rename";
            progress.sql = AttachmentSql::StatementFailed;
            let affected = resources
                .write(
                    &db,
                    "rename",
                    vec![text(&item_id), text(&attachment_id), text(&display_name)],
                )
                .map_err(|e| progress.data(e))?;
            progress.sql = AttachmentSql::Returned;
            if affected == 0 {
                return Err(progress.missing());
            }
            Ok(AttachmentResult::Done)
        }
        AttachmentRequest::SaveState { input } => {
            let Some(AttachmentPlanV1::SaveReadingState { bounded_zoom, .. }) = plan else {
                return Err(progress.fail("plugin_host_response_mismatch", "阅读状态准备缺失。"));
            };
            let db = store.connection().map_err(|e| progress.owner(e))?;
            let mut progress = progress;
            progress.phase = "reading-state-save";
            progress.sql = AttachmentSql::StatementFailed;
            resources
                .write(
                    &db,
                    "reading-state-save",
                    vec![
                        text(&input.item_id),
                        text(&input.attachment_id),
                        DataValue::Integer(i64::from(input.page_index)),
                        DataValue::Real(bounded_zoom.to_f64()),
                        optional(input.view_state_json.as_deref()),
                    ],
                )
                .map_err(|e| progress.data(e))?;
            Ok(AttachmentResult::Done)
        }
        AttachmentRequest::GetState {
            item_id,
            attachment_id,
        } => {
            let db = store.connection().map_err(|e| progress.owner(e))?;
            let rows = resources
                .read(
                    &db,
                    "reading-state-get",
                    vec![text(&item_id), text(&attachment_id)],
                )
                .map_err(|e| progress.data(e))?;
            let state = rows
                .rows
                .first()
                .map(|row| {
                    let [DataValue::Integer(page), DataValue::Text(zoom), json] = row.as_slice()
                    else {
                        return Err(progress.data(resources.shape_error("reading-state-get")));
                    };
                    let bits = F64BitsV1::parse(zoom.clone())
                        .map_err(|_| progress.data(resources.shape_error("reading-state-get")))?;
                    let json = match json {
                        DataValue::Null => None,
                        DataValue::Text(value) => Some(value.clone()),
                        _ => return Err(progress.data(resources.shape_error("reading-state-get"))),
                    };
                    let page_index = i32::try_from(*page)
                        .map_err(|_| progress.data(resources.shape_error("reading-state-get")))?;
                    Ok(ReadingPosition {
                        page_index,
                        zoom: bits.to_f64(),
                        view_state_json: json,
                    })
                })
                .transpose()?;
            Ok(AttachmentResult::State(state))
        }
        AttachmentRequest::IsIndexed { attachment_id } => {
            let db = store.connection().map_err(|e| progress.owner(e))?;
            scalar_bool(
                &db,
                resources,
                "is-indexed",
                vec![text(&attachment_id)],
                &progress,
            )
            .map(AttachmentResult::Indexed)
        }
        AttachmentRequest::Index {
            item_id,
            attachment_id,
            pages,
        } => {
            let mut db = store.connection().map_err(|e| progress.owner(e))?;
            transaction::run(&mut db, |tx, progress| {
                progress.phase = "index-job-running";
                let count = i64::try_from(pages.len()).map_err(|cause| {
                    progress.sql_error(&rusqlite::Error::ToSqlConversionFailure(Box::new(cause)))
                })?;
                resources
                    .write(
                        tx,
                        "index-job-running",
                        vec![text(&attachment_id), DataValue::Integer(count)],
                    )
                    .map_err(|e| progress.data(e))?;
                progress.phase = "index-pages-delete";
                resources
                    .write(tx, "index-pages-delete", vec![text(&attachment_id)])
                    .map_err(|e| progress.data(e))?;
                for page in pages {
                    progress.phase = "index-page-insert";
                    resources
                        .write(
                            tx,
                            "index-page-insert",
                            vec![
                                text(&attachment_id),
                                DataValue::Integer(i64::from(page.page_index)),
                                text(&page.text),
                            ],
                        )
                        .map_err(|e| progress.data(e))?;
                }
                progress.phase = "index-fts";
                resources
                    .write(tx, "index-fts", vec![text(&item_id)])
                    .map_err(|e| progress.data(e))?;
                progress.phase = "index-job-completed";
                resources
                    .write(tx, "index-job-completed", vec![text(&attachment_id)])
                    .map_err(|e| progress.data(e))?;
                Ok(())
            })?;
            Ok(AttachmentResult::Done)
        }
    }
}

pub(super) fn relative_path(
    db: &Connection,
    resources: &AttachmentResources,
    id: &str,
    progress: &AttachmentProgress,
) -> Result<String> {
    let rows = resources
        .read(db, "content-path", vec![text(id)])
        .map_err(|e| progress.data(e))?;
    match rows.rows.first().map(Vec::as_slice) {
        Some([DataValue::Text(value)]) => Ok(value.clone()),
        None => Err(progress.missing()),
        _ => Err(progress.data(resources.shape_error("content-path"))),
    }
}

pub(super) fn scalar_bool(
    db: &Connection,
    resources: &AttachmentResources,
    key: &str,
    args: Vec<DataValue>,
    progress: &AttachmentProgress,
) -> Result<bool> {
    let rows = resources
        .read(db, key, args)
        .map_err(|e| progress.data(e))?;
    match rows.rows.first().map(Vec::as_slice) {
        Some([DataValue::Integer(value)]) => Ok(*value != 0),
        _ => Err(progress.data(resources.shape_error(key))),
    }
}

pub(crate) fn attach(
    db: &Connection,
    store: &AppStore,
    resources: &AttachmentResources,
    item_id: &str,
    path: &str,
) -> Result<CatalogAttachment> {
    let source = store.pdf_objects().file_source(std::path::Path::new(path));
    attach_source(db, store, resources, item_id, &source)
}

pub(crate) fn attach_source(
    db: &Connection,
    store: &AppStore,
    resources: &AttachmentResources,
    item_id: &str,
    source: &crate::content::PdfImportSource,
) -> Result<CatalogAttachment> {
    let mut progress = AttachmentProgress::new("attach-active-parent");
    if !scalar_bool(
        db,
        resources,
        "active-parent",
        vec![text(item_id)],
        &progress,
    )? {
        return Err(progress.missing());
    }
    let objects = store.pdf_objects();
    let objects = objects.in_snapshot(db);
    progress.phase = "attach-probe";
    let proof = objects
        .probe_pdf_source(source)
        .map_err(|e| progress.pdf(e))?;
    progress.phase = "attach-duplicate";
    if scalar_bool(
        db,
        resources,
        "same-item-hash",
        vec![text(item_id), text(proof.sha256())],
        &progress,
    )? {
        return Err(progress.fail(
            "attachment_duplicate",
            "此文献已包含内容相同的 PDF，不能重复添加",
        ));
    }
    progress.phase = "content-publish";
    let published = objects.publish_pdf(&proof).map_err(|cause| {
        if matches!(&cause, crate::content::PdfObjectError::Database(_)) {
            progress.sql = AttachmentSql::StatementFailed;
        }
        progress.pdf(cause)
    })?;
    progress.sql = AttachmentSql::Returned;
    progress.content_publish_returned = true;
    let id = uuid::Uuid::new_v4().to_string();
    progress.attachment_id = Some(id.clone());
    let metadata = proof.source_metadata();
    progress.phase = "attach-insert";
    progress.sql = AttachmentSql::StatementFailed;
    resources
        .write(
            db,
            "insert",
            vec![
                text(&id),
                text(item_id),
                text(&published.sha256),
                text(&metadata.display_name),
                text(&metadata.source_path),
            ],
        )
        .map_err(|e| progress.data(e))?;
    progress.sql = AttachmentSql::Returned;
    progress.attachment_insert_returned = true;
    progress.phase = "attach-other-items";
    let rows = resources
        .read(
            db,
            "other-items",
            vec![text(&published.sha256), text(item_id)],
        )
        .map_err(|e| progress.data(e))?;
    for row in rows.rows {
        let [DataValue::Text(other)] = row.as_slice() else {
            return Err(progress.data(resources.shape_error("other-items")));
        };
        progress.phase = "attach-duplicate-insert";
        progress.sql = AttachmentSql::StatementFailed;
        resources
            .write(
                db,
                "duplicate-insert",
                vec![
                    text(&uuid::Uuid::new_v4().to_string()),
                    text(item_id),
                    text(other),
                    text("content_hash"),
                    DataValue::Real(1.0),
                ],
            )
            .map_err(|e| progress.data(e))?;
        progress.sql = AttachmentSql::Returned;
        progress.duplicate_insert_returns += 1;
    }
    Ok(CatalogAttachment {
        id,
        display_name: metadata.display_name,
        mime_type: "application/pdf".into(),
        byte_size: published.byte_size,
        content_sha256: published.sha256,
    })
}
