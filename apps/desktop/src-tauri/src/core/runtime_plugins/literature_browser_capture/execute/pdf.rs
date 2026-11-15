use super::*;
use research_workbench_native_sdk::catalog::CatalogAttachment;

pub(super) fn save_attachment(
    store: &AppStore,
    resources: &CaptureStorage,
    scope: &str,
    plan: PreparedCaptureAttachmentV1,
    bytes: &[u8],
) -> Result<Value> {
    let _content = store.content_write()?;
    let mut db = store.connection()?;
    if let Some(receipt) = attachment_receipt(&db, &resources.sql, scope, &plan)? {
        return Ok(receipt);
    }
    let id: String = db
        .query_row(
            resources.sql.sql("active-parent"),
            params![scope, plan.session_id, plan.parent_external_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(CaptureFailure::NotFound)?;
    store.ensure_content_import_capacity(bytes.len() as u64)?;
    pdf_transaction(store, &mut db, bytes, |tx| {
        let attachment = import_pdf(store, tx, resources, &id, &plan, bytes)?;
        let receipt = json!({"id":plan.original_id,
            "itemID":id,"attachmentID":attachment.id,"contentSha256":attachment.content_sha256,"byteSize":attachment.byte_size});
        record_attachment(tx, &resources.sql, scope, &plan, &receipt)?;
        Ok(receipt)
    })
}

pub(super) fn save_standalone(
    store: &AppStore,
    resources: &CaptureStorage,
    scope: &str,
    plan: PreparedStandaloneCaptureV1,
    bytes: &[u8],
) -> Result<Value> {
    let _content = store.content_write()?;
    let mut db = store.connection()?;
    let attachment_plan = &plan.attachment;
    if let Some(receipt) = stored_receipt(
        &db,
        &resources.sql,
        scope,
        &attachment_plan.session_id,
        &attachment_plan.request_sha256,
    )? {
        return Ok(receipt);
    }
    store.ensure_content_import_capacity(bytes.len() as u64)?;
    pdf_transaction(store, &mut db, bytes, |tx| {
        insert_session(
            tx,
            &resources.sql,
            scope,
            &attachment_plan.session_id,
            "standalone",
            &attachment_plan.metadata_json,
            &attachment_plan.request_sha256,
        )?;
        let save = save_plan(plan.save)?;
        let id = uuid::Uuid::new_v4().to_string();
        save::save_in_transaction(tx, &resources.catalog, &save, &id)?;
        record_item(
            tx,
            &resources.sql,
            scope,
            &attachment_plan.session_id,
            "standalone",
            &id,
            &plan.raw,
        )?;
        tx.execute(
            resources.sql.sql("provenance-source"),
            params![id, plan.source_reference.to_string()],
        )?;
        let attachment = import_pdf(store, tx, resources, &id, attachment_plan, bytes)?;
        let receipt = json!({"sessionID":attachment_plan.session_id,"itemID":id,"attachmentID":attachment.id,
            "contentSha256":attachment.content_sha256,"byteSize":attachment.byte_size,"canRecognize":false,"metadataPending":true});
        record_attachment(tx, &resources.sql, scope, attachment_plan, &receipt)?;
        finish_session(
            tx,
            &resources.sql,
            scope,
            &attachment_plan.session_id,
            &receipt,
        )?;
        Ok(receipt)
    })
}
fn pdf_transaction(
    store: &AppStore,
    db: &mut crate::data_interface::DatabaseGuard<'_>,
    bytes: &[u8],
    operation: impl FnOnce(&Transaction<'_>) -> Result<Value>,
) -> Result<Value> {
    let objects = store.pdf_objects();
    let checkpoint = objects.checkpoint_pdf_bytes(bytes);
    let result = transaction::run(db, operation);
    if result.is_err() {
        objects
            .in_snapshot(db)
            .cleanup_unregistered_after_error(&checkpoint);
    }
    result
}
fn attachment_receipt(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    plan: &PreparedCaptureAttachmentV1,
) -> Result<Option<Value>> {
    let saved: Option<(String, String)> = db
        .query_row(
            sql.sql("attachment-receipt"),
            params![scope, plan.session_id, plan.external_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    saved
        .map(|(hash, receipt)| {
            if hash != plan.request_sha256 {
                return Err(CaptureFailure::Conflict);
            }
            decode_receipt(&receipt)
        })
        .transpose()
}
fn import_pdf(
    store: &AppStore,
    tx: &Transaction<'_>,
    resources: &CaptureStorage,
    id: &str,
    plan: &PreparedCaptureAttachmentV1,
    bytes: &[u8],
) -> Result<CatalogAttachment> {
    let staged = store
        .pdf_objects()
        .stage_pdf_bytes(bytes)
        .map_err(|_| CaptureFailure::Storage)?;
    let mut attachment =
        literature_attachments::attach_source(tx, store, &resources.attachments, id, &staged)
            .map_err(|error| match error.code.as_str() {
                "not_found" => CaptureFailure::NotFound,
                "invalid_input" => CaptureFailure::InvalidInput,
                _ => CaptureFailure::Technical(error.message),
            })?;
    tx.execute(
        resources.sql.sql("attachment-label"),
        params![attachment.id, plan.display_name, plan.source_url],
    )?;
    attachment.display_name = plan.display_name.clone();
    Ok(attachment)
}
fn record_attachment(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    plan: &PreparedCaptureAttachmentV1,
    receipt: &Value,
) -> Result<()> {
    db.execute(
        sql.sql("attachment-insert"),
        params![
            scope,
            plan.session_id,
            plan.external_id,
            receipt["itemID"].as_str(),
            receipt["attachmentID"].as_str(),
            plan.request_sha256,
            plan.metadata_json,
            receipt["contentSha256"].as_str(),
            receipt.to_string()
        ],
    )?;
    Ok(())
}
