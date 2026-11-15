use super::*;
use crate::runtime_plugins::{literature_attachments::AttachmentSql, native::NativeHostError};

pub(super) fn require(condition: bool, message: &'static str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
pub(super) fn diagnostic(error: &NativeHostError, key: &str, code: &str) -> Result<Value, Error> {
    require(
        error.code == code
            && error.package_key.as_deref() == Some(key)
            && error.cleanup_error.is_none(),
        "attachment rejection lost code/package or actual cleanup failed",
    )?;
    uuid::Uuid::parse_str(
        error
            .instance_id
            .as_deref()
            .ok_or("attachment error lost instance")?,
    )?;
    let value = serde_json::to_value(error)?;
    require(
        !serde_json::to_string(&value)?.contains("private synthetic attachment"),
        "attachment diagnostic leaked private SQL fixture text",
    )?;
    Ok(value)
}
async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: AttachmentRequest,
    code: &str,
) -> Result<Value, Error> {
    match service
        .attachments(store.clone(), key.into(), request)
        .await
    {
        Err(error) => diagnostic(&error, key, code),
        Ok(_) => Err("attachment rejection unexpectedly succeeded".into()),
    }
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let bytes = fixture::pdf("Synthetic original PDF");
    let source = fixture::source(directory, "original.pdf", &bytes)?;
    let first = attach(store, service, key, fixture::PAPER, &source).await?;
    require(
        first.display_name == "original.pdf"
            && first.mime_type == "application/pdf"
            && first.byte_size == bytes.len() as u64
            && first.content_sha256 == format!("{:x}", Sha256::digest(&bytes)),
        "actual attachment identity differs from source PDF",
    )?;
    let AttachmentResult::Bytes(actual) = call(
        store,
        service,
        key,
        AttachmentRequest::Read {
            attachment_id: first.id.clone(),
        },
    )
    .await?
    else {
        return Err("binary result kind".into());
    };
    require(actual == bytes, "binary read changed exact PDF bytes")?;
    let before_duplicate = readback::snapshot(store)?;
    let duplicate = rejected(
        store,
        service,
        key,
        AttachmentRequest::Attach {
            item_id: fixture::PAPER.into(),
            path: source.to_str().ok_or("source path")?.into(),
        },
        "attachment_duplicate",
    )
    .await?;
    require(
        readback::snapshot(store)? == before_duplicate,
        "same-item rejection changed SQL",
    )?;
    let shared = attach(store, service, key, fixture::OTHER, &source).await?;
    require(
        shared.content_sha256 == first.content_sha256,
        "shared object identity changed",
    )?;
    done(
        store,
        service,
        key,
        AttachmentRequest::Rename {
            item_id: fixture::PAPER.into(),
            attachment_id: first.id.clone(),
            display_name: "  Synthetic renamed.pdf  ".into(),
        },
    )
    .await?;
    let renamed: (String, String, String) = store.connection()?.query_row(
        "select display_name,content_sha256,source_path from attachments where id=?1",
        [&first.id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    require(
        renamed.0 == "Synthetic renamed.pdf"
            && renamed.1 == first.content_sha256
            && renamed.2 == source.to_str().ok_or("source path")?,
        "rename changed original identity/source or did not trim display name",
    )?;
    let raw = " {\"schemaVersion\":2,\"pageIndex\":4,\"scale\":{\"kind\":\"page-fit\"},\"scrollMode\":\"vertical\",\"spreadMode\":\"none\"} ";
    done(
        store,
        service,
        key,
        AttachmentRequest::SaveState {
            input: SaveReadingStateInputV1 {
                item_id: fixture::PAPER.into(),
                attachment_id: first.id.clone(),
                page_index: -7,
                zoom: F64BitsV1::from_f64(9.0),
                view_state_json: Some(raw.into()),
            },
        },
    )
    .await?;
    let AttachmentResult::State(Some(state)) = call(
        store,
        service,
        key,
        AttachmentRequest::GetState {
            item_id: fixture::PAPER.into(),
            attachment_id: first.id.clone(),
        },
    )
    .await?
    else {
        return Err("state result kind".into());
    };
    require(
        state.page_index == 0
            && state.zoom.to_bits() == 3.0_f64.to_bits()
            && state.view_state_json.as_deref() == Some(raw),
        "reading state lost original page/zoom/raw JSON",
    )?;
    let index = |pages| AttachmentRequest::Index {
        item_id: fixture::PAPER.into(),
        attachment_id: first.id.clone(),
        pages,
    };
    done(
        store,
        service,
        key,
        index(vec![
            PageText {
                page_index: 9,
                text: "Later synthetic page".into(),
            },
            PageText {
                page_index: -3,
                text: "Clamped synthetic page".into(),
            },
        ]),
    )
    .await?;
    let AttachmentResult::Indexed(indexed) = call(
        store,
        service,
        key,
        AttachmentRequest::IsIndexed {
            attachment_id: first.id.clone(),
        },
    )
    .await?
    else {
        return Err("index status result kind".into());
    };
    require(indexed, "actual text-index completion missing")?;
    let before_rollback = readback::snapshot(store)?;
    let pages = before_rollback["pages"]
        .as_array()
        .ok_or("missing actual page projection")?;
    require(
        pages.len() == 2
            && pages[0]["page_index"] == 0
            && pages[0]["page_text"] == "Clamped synthetic page"
            && pages[1]["page_index"] == 9
            && pages[1]["page_text"] == "Later synthetic page"
            && before_rollback["indexJobs"][0]["state"] == "completed"
            && before_rollback["indexJobs"][0]["page_count"] == 2,
        "index did not preserve original page order/clamping/job count",
    )?;
    let rollback = rejected(
        store,
        service,
        key,
        index(vec![
            PageText {
                page_index: -1,
                text: "Private replacement page".into(),
            },
            PageText {
                page_index: 0,
                text: "Conflicting replacement page".into(),
            },
        ]),
        "storage_error",
    )
    .await?;
    require(
        rollback["attachments"]["sql"] == "rolledBack"
            && readback::snapshot(store)? == before_rollback,
        "index collision did not roll back pages/job/FTS",
    )?;
    let missing = rejected(
        store,
        service,
        key,
        AttachmentRequest::Read {
            attachment_id: "synthetic-missing".into(),
        },
        "not_found",
    )
    .await?;
    let original_path: String = store.connection()?.query_row(
        "select relative_path from content_objects where sha256=?1",
        [&first.content_sha256],
        |r| r.get(0),
    )?;
    store.connection()?.execute(
        "update content_objects set relative_path='../synthetic-outside.pdf' where sha256=?1",
        [&first.content_sha256],
    )?;
    let unsafe_path = rejected(
        store,
        service,
        key,
        AttachmentRequest::Read {
            attachment_id: first.id.clone(),
        },
        "storage_error",
    )
    .await?;
    store.connection()?.execute(
        "update content_objects set relative_path=?2 where sha256=?1",
        rusqlite::params![first.content_sha256, original_path],
    )?;
    fixture::protect(store, &first)?;
    let before_protected = readback::snapshot(store)?;
    let protected = rejected(
        store,
        service,
        key,
        AttachmentRequest::Remove {
            item_id: fixture::PAPER.into(),
            attachment_id: first.id.clone(),
        },
        "attachment_in_use",
    )
    .await?;
    require(
        protected["attachments"]["sql"] == "rolledBack"
            && readback::snapshot(store)? == before_protected,
        "protected remove changed original evidence/anchor prefix",
    )?;
    let unique_bytes = fixture::pdf("Synthetic deleted PDF");
    let unique_source = fixture::source(directory, "delete.pdf", &unique_bytes)?;
    let deleted = attach(store, service, key, fixture::OTHER, &unique_source).await?;
    let deleted_file = fixture::stored(store, &deleted.id)?;
    let deleted_path = readback::files(store)?;
    let removed = remove(store, service, key, fixture::OTHER, &deleted.id).await?;
    require(
        removed.file_status == "deleted"
            && removed.warning.is_none()
            && unique_source.exists()
            && !deleted_file.exists(),
        "real remove lost original source or did not unlink object",
    )?;
    let absent: i64 = store.connection()?.query_row(
        "select count(*) from attachments where id=?1",
        [&deleted.id],
        |r| r.get(0),
    )?;
    require(absent == 0, "deleted attachment metadata remained")?;
    let missing_source = fixture::source(
        directory,
        "missing-file.pdf",
        &fixture::pdf("Synthetic missing object"),
    )?;
    let missing_file = attach(store, service, key, fixture::OTHER, &missing_source).await?;
    std::fs::remove_file(fixture::stored(store, &missing_file.id)?)?;
    let removed_missing = remove(store, service, key, fixture::OTHER, &missing_file.id).await?;
    require(
        removed_missing.file_status == "missing"
            && removed_missing.warning.is_none()
            && missing_source.exists(),
        "missing physical object did not retain original metadata cleanup semantics",
    )?;
    let retained = remove(store, service, key, fixture::OTHER, &shared.id).await?;
    require(
        retained.file_status == "retained" && retained.warning.is_none(),
        "shared object should remain retained",
    )?;
    let partial = partial_failure(store, service, key, &source).await?;
    let pdf_job = super::pdf::run(store, service, key, directory).await?;
    let identity = super::identity::run(store, service, key, &first.id).await?;
    Ok(
        json!({"originalAttachment":first,"originalBytesSha256":format!("{:x}",Sha256::digest(&bytes)),
        "originalByteSize":bytes.len(),"state":state,"duplicate":duplicate,"indexRollback":rollback,
        "missing":missing,"unsafePath":unsafe_path,"protectedRemove":protected,
        "deletedAttachment":deleted,"filesBeforeDelete":deleted_path,"deleteResult":removed,
        "sharedRemoveResult":retained,"missingFileRemoveResult":removed_missing,"partialFailure":partial,"pdfImport":pdf_job,"shutdownIdentity":identity,
        "revealVerified":false,"revealUnverifiedReason":"No AppHandle in the existing finite CLI; no fake opener used",
        "methodScope":"8 of 9 original attachment backend methods (Reveal excluded), PDF start and shared jobs Get/Cancel",
        "uiOrPdfJsRenderingVerified":false}),
    )
}

async fn partial_failure(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    source: &Path,
) -> Result<Value, Error> {
    store.connection()?.execute_batch("create temp trigger synthetic_attachment_cli_duplicate_failure before insert on duplicate_candidates
        begin select raise(abort,'private synthetic attachment duplicate failure'); end;")?;
    let result = service
        .attachments(
            store.clone(),
            key.into(),
            AttachmentRequest::Attach {
                item_id: fixture::PARTIAL.into(),
                path: source.to_str().ok_or("source path")?.into(),
            },
        )
        .await;
    store
        .connection()?
        .execute_batch("drop trigger synthetic_attachment_cli_duplicate_failure;")?;
    let error = result
        .err()
        .ok_or("partial attachment failure unexpectedly succeeded")?;
    let value = diagnostic(&error, key, "storage_error")?;
    let prefix = error
        .attachments
        .as_ref()
        .ok_or("partial failure lost progress")?;
    require(
        prefix.progress.phase == "attach-duplicate-insert"
            && prefix.progress.sql == AttachmentSql::StatementFailed
            && prefix.progress.content_publish_returned
            && prefix.progress.attachment_insert_returned
            && prefix.progress.duplicate_insert_returns == 0
            && error.request_id.is_none(),
        "partial failure misstated returned prefix or inherited old request",
    )?;
    let id = prefix
        .progress
        .attachment_id
        .as_deref()
        .ok_or("partial failure lost actual attachment id")?;
    let actual: String = store.connection()?.query_row(
        "select item_id from attachments where id=?1",
        [id],
        |r| r.get(0),
    )?;
    require(
        actual == fixture::PARTIAL,
        "autocommitted attachment disappeared after later error",
    )?;
    Ok(value)
}
