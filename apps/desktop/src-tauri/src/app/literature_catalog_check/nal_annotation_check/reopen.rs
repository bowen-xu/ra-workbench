use super::*;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original: Value = serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    if original["status"] != "passed-nal-annotation-native-core-only"
        || original["packageKey"] != key
    {
        return Err("cold annotation reopen requires this exact successful package receipt".into());
    }
    let expected: NalAnnotation = serde_json::from_value(original["annotation"].clone())?;
    let old_note = original["oldNoteId"]
        .as_str()
        .ok_or("original synthetic Core note id missing")?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"]
        || !stored_annotation(store, &expected)?
        || fixture::note(store, old_note)? != original["oldNoteDocument"]
        || fixture::note(store, &expected.note_page_id)? != original["newNoteDocument"]
        || readback::metadata(store)? != original["metadata"]
    {
        return Err("fresh SQL/log reopen changed annotation, documents or actual startup metadata/queue before Native call".into());
    }
    // This is a new explicit Existing lookup with a different title, not a
    // persisted write request replay. No seed, repair or write is performed.
    let lookup_title = "Synthetic cold-open explicit Existing lookup";
    let actual = ensure(
        store,
        service,
        key,
        NalAnnotationRequest {
            target_kind: expected.target_kind.clone(),
            target_id: expected.target_id.clone(),
            title: lookup_title.into(),
        },
    )
    .await?;
    let after = readback::snapshot(store)?;
    if actual != expected
        || !stored_annotation(store, &actual)?
        || after != before
        || fixture::note(store, old_note)? != original["oldNoteDocument"]
        || fixture::note(store, &actual.note_page_id)? != original["newNoteDocument"]
        || readback::metadata(store)? != original["metadata"]
    {
        return Err("cold Existing lookup changed original annotation identity, Core notes or complete persistent metadata".into());
    }
    Ok(
        json!({"annotation":actual,"sqlSnapshot":after,"metadata":original["metadata"],
        "oldNoteId":old_note,"oldNoteDocument":original["oldNoteDocument"],
        "newNoteDocument":original["newNoteDocument"],"sameCompleteSqlSnapshot":true,
        "sameCoreNotes":true,"sameAnnotationIdentity":true,"explicitExistingCall":true,
        "lookupTitle":lookup_title,"seededOrReplayed":false,"originalReceipt":"receipt.json",
        "coreOnly":true}),
    )
}
