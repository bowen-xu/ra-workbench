//! Real ensure calls with synthetic startup, Core Note and cold-open evidence.
use super::*;
use research_workbench_native_sdk::nal_annotation::{NalAnnotation, NalAnnotationRequest};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

type Error = Box<dyn std::error::Error>;

#[path = "nal_annotation_check/reopen.rs"]
mod cold;
#[path = "nal_annotation_check/failures.rs"]
mod failures;
use super::{nal_bootstrap_fixture as fixture, nal_bootstrap_readback as readback};

const TARGET: &str = "bootstrap-endpoint";
const CREATED_BODY: &str = "Synthetic Core-owned annotation note body";

fn request(target: &str, title: impl Into<String>) -> NalAnnotationRequest {
    NalAnnotationRequest {
        target_kind: "concept".into(),
        target_id: target.into(),
        title: title.into(),
    }
}

async fn ensure(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: NalAnnotationRequest,
) -> Result<NalAnnotation, Error> {
    Ok(service
        .ensure_nal_annotation(store.shared_agent_connection(), key.into(), request)
        .await?)
}

fn save_core_body(store: &AppStore, page: &str) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    let mut document = crate::notes::load_document_value(&tx, page)?;
    let first = document
        .blocks
        .first_mut()
        .ok_or("Core-created annotation note has no initial block")?;
    first.content =
        json!({"kind":"inline","items":[{"type":"text","text":CREATED_BODY,"styles":[]}]});
    let expected_revision: i64 = tx.query_row(
        "select revision from note_pages where id=?1",
        [page],
        |row| row.get(0),
    )?;
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision,
            save_reason: "manual".into(),
        },
    )?;
    tx.commit()?;
    drop(db);
    fixture::note(store, page)
}

fn stored_annotation(store: &AppStore, annotation: &NalAnnotation) -> Result<bool, Error> {
    let db = store.connection()?;
    Ok(annotation.target_kind == "concept" && db.query_row(
        "select exists(select 1 from nal_annotations where id=?1 and concept_id=?2 and statement_id is null and note_page_id=?3)",
        rusqlite::params![annotation.id,annotation.target_id,annotation.note_page_id],
        |row| row.get(0),
    )?)
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let invalid = failures::invalid_before_sql(store, service, key).await?;
    let (old_note, queue_attempts) = fixture::seed(store)?;
    let old_document = fixture::note(store, &old_note)?;
    let note_attempts = failures::observe_notes(store)?;
    let malformed_before = readback::snapshot(store)?;
    let malformed = failures::call(
        store,
        service,
        key,
        request(TARGET, "Synthetic malformed-startup annotation"),
        "invalid_input",
        "关系数据结构无效",
        "rolledBack",
    )
    .await?;
    let malformed_queue_attempts = queue_attempts.load(Ordering::SeqCst);
    if malformed_queue_attempts == 0
        || note_attempts.load(Ordering::SeqCst) != 0
        || readback::snapshot(store)? != malformed_before
        || fixture::note(store, &old_note)? != old_document
    {
        return Err(
            "malformed startup did not roll back full startup/queue before creating a Core note"
                .into(),
        );
    }
    // Only this synthetic malformed row is repaired. Its old NAL annotation
    // cascades, while its Core page, document and history remain untouched.
    if store
        .connection()?
        .execute("delete from nal_statements where id=?1", [fixture::BAD])?
        != 1
    {
        return Err("synthetic malformed Fact repair did not affect one row".into());
    }
    let link_failure =
        failures::write_failure(store, service, key, true, &queue_attempts, &note_attempts).await?;
    let revision_failure =
        failures::write_failure(store, service, key, false, &queue_attempts, &note_attempts)
            .await?;
    if fixture::note(store, &old_note)? != old_document {
        return Err("failed ensures altered the preexisting Core document".into());
    }

    let display_title = "界".repeat(240);
    let notes_before_create = note_attempts.load(Ordering::SeqCst);
    let annotation = ensure(
        store,
        service,
        key,
        request(TARGET, format!(" \t{display_title}\n ")),
    )
    .await?;
    let created = fixture::note(store, &annotation.note_page_id)?;
    if annotation.target_kind != "concept"
        || annotation.target_id != TARGET
        || annotation.id.is_empty()
        || !stored_annotation(store, &annotation)?
        || annotation.note_page_id == old_note
        || created["title"] != display_title
        || note_attempts.load(Ordering::SeqCst) != notes_before_create + 1
        || readback::scalar(store, "select count(*) from note_pages")? != 2
        || readback::scalar(
            store,
            "select legacy_imported=1 and relation_semantics_version=1 from nal_state where id=1",
        )? != 1
    {
        return Err("successful ensure did not return its original annotation and one Core-created trimmed-title note".into());
    }
    let new_document = save_core_body(store, &annotation.note_page_id)?;
    if new_document["blocks"][0]["content"]["items"][0]["text"] != CREATED_BODY {
        return Err("original Core note writer did not preserve the synthetic body".into());
    }
    let metadata = readback::metadata(store)?;
    if store.connection()?.execute(
        "update note_pages set status='trashed' where id=?1",
        [&annotation.note_page_id],
    )? != 1
    {
        return Err("synthetic annotation page status update failed".into());
    }
    let stable = readback::snapshot(store)?;
    let existing_note_attempts = note_attempts.load(Ordering::SeqCst);
    let mut long_title_failures = vec![];
    for length in [241, 300] {
        let title = "界".repeat(length);
        if ensure(store, service, key, request(TARGET, title.clone())).await? != annotation
            || readback::snapshot(store)? != stable
        {
            return Err(
                "Existing trashed annotation was recreated or changed by a legal NAL long title"
                    .into(),
            );
        }
        // This active, imported target has no annotation. Only Create reaches
        // Core's trimmed 240-character limit; Existing never invokes that writer.
        long_title_failures.push(
            failures::call(
                store,
                service,
                key,
                request(fixture::PLACEHOLDER, title),
                "invalid_input",
                "笔记创建失败，请重试",
                "rolledBack",
            )
            .await?,
        );
        if readback::snapshot(store)? != stable
            || note_attempts.load(Ordering::SeqCst) != existing_note_attempts
        {
            return Err(
                "long-title Create failure changed the committed annotation/startup/Core note"
                    .into(),
            );
        }
    }
    if fixture::note(store, &old_note)? != old_document
        || fixture::note(store, &annotation.note_page_id)? != new_document
        || readback::metadata(store)? != metadata
    {
        return Err(
            "annotation calls changed retained documents or bootstrap metadata/queue".into(),
        );
    }
    Ok(json!({
        "annotation":annotation,"oldNoteId":old_note,"oldNoteDocument":old_document,
        "newNoteDocument":new_document,"sqlSnapshot":stable,"metadata":metadata,
        "invalidRequestFailure":invalid,"malformedStartupFailure":malformed,
        "linkRollbackFailure":link_failure,"finalRevisionRollbackFailure":revision_failure,
        "createLongTitleFailures":long_title_failures,
        "malformedStartupQueueAttempts":malformed_queue_attempts,
        "wholeStartupAndCoreNoteRolledBack":true,"createdNoteSavedByCore":true,
        "existingTrashedLongTitlePreserved":true,"existingDidNotCreatePage":true,
        "coreOnly":true
    }))
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    cold::run(store, service, key, directory).await
}
