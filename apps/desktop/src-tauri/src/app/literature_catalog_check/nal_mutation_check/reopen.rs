use super::*;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original: Value = serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    if original["status"] != "passed-nal-mutation-native-core-only"
        || original["packageKey"] != key
        || original["allSevenOriginalVariants"] != true
        || original["explicitReadCleaned"] != true
    {
        return Err(
            "cold mutation reopen requires this exact successful, explicitly cleaned receipt"
                .into(),
        );
    }
    let expected: NalState = serde_json::from_value(original["state"].clone())?;
    let annotation: NalAnnotation = serde_json::from_value(original["annotation"].clone())?;
    let note = original["noteId"]
        .as_str()
        .ok_or("original synthetic Core note id missing")?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"]
        || fixture::note(store, note)? != original["noteDocument"]
        || readback::metadata(store)? != original["metadata"]
    {
        return Err(
            "fresh SQL/log reopen changed the frozen SQL metadata or Core Note before Native read"
                .into(),
        );
    }
    // This explicit completed read must not repeat the prior mutation request,
    // seed the DB, repair the fixture or execute a no-op mutation as a replay.
    let state = read(store, service, key).await?;
    let after = readback::snapshot(store)?;
    if state != expected
        || state.annotations != vec![annotation.clone()]
        || after != before
        || fixture::note(store, note)? != original["noteDocument"]
        || readback::metadata(store)? != original["metadata"]
    {
        return Err("cold Native read changed complete State, persistent annotation, metadata or original Core Note".into());
    }
    Ok(
        json!({"state":state,"sqlSnapshot":after,"annotation":annotation,
        "noteId":note,"noteDocument":original["noteDocument"],"metadata":original["metadata"],
        "sameCompleteState":true,"sameCompleteSqlSnapshot":true,"sameCoreNote":true,
        "sameAnnotationIdentity":true,"seededOrReplayed":false,"originalReceipt":"receipt.json",
        "coreOnly":true}),
    )
}
