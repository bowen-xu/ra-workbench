//! Real finite native preparation, original Core transactions and SQL/log cold reads.
use super::*;
use research_workbench_native_sdk::duplicate_review::MergeOperation;

#[path = "duplicate_merge_check/fixture.rs"]
mod fixture;
#[path = "duplicate_merge_check/readback.rs"]
mod readback;

async fn merge(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<MergeOperation, Error> {
    let case = fixture::case();
    Ok(service
        .merge_duplicate_candidate(
            store.shared_agent_connection(),
            key.into(),
            case.candidate,
            case.survivor,
        )
        .await?)
}

async fn absent(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let case = fixture::case();
    let error = service
        .merge_duplicate_candidate(
            store.shared_agent_connection(),
            key.into(),
            case.candidate,
            case.survivor,
        )
        .await
        .err()
        .ok_or("decided candidate unexpectedly merged again")?;
    let wire = serde_json::to_value(&error)?;
    if error.code != "not_found"
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || wire["duplicateMerge"]["outcome"] != "notStarted"
        || wire["duplicateMerge"]["problem"]["phase"] != "header"
        || error.operation_id.is_some()
        || error.request_id.is_some()
        || before != readback::snapshot(store)?
    {
        return Err(format!(
            "decided-candidate rejection lost its actual outcome/identity: {wire}"
        )
        .into());
    }
    readback::assert_note(store, note)?;
    Ok(wire)
}

async fn late_failure(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    store.connection()?.execute_batch(
        "create temp trigger merge_cli_history_failure before insert on merge_operations
        begin select raise(abort,'private synthetic merge history fault');end;",
    )?;
    let before = readback::snapshot(store)?;
    let case = fixture::case();
    let result = service
        .merge_duplicate_candidate(
            store.shared_agent_connection(),
            key.into(),
            case.candidate,
            case.survivor,
        )
        .await;
    store
        .connection()?
        .execute_batch("drop trigger merge_cli_history_failure")?;
    let error = result
        .err()
        .ok_or("late merge history failure unexpectedly committed")?;
    // Keep the original structured native error, including its real operation ID.
    let wire = serde_json::to_value(&error)?;
    if wire["code"] != "storage_error"
        || wire["pluginId"] != "literature_library"
        || wire["packageKey"] != key
        || wire["instanceId"].as_str().is_none()
        || wire["requestId"].as_str().is_none()
        || wire["operationId"].as_str().is_none()
        || wire["operationId"] != wire["duplicateMerge"]["operationId"]
        || wire["duplicateMerge"]["outcome"] != "rolledBack"
        || wire["duplicateMerge"]["problem"]["phase"] != "history"
        || wire["duplicateMerge"]["problem"]["cause"]["diagnostic"]["sqliteExtendedCode"] != 1811
        || wire["duplicateMerge"].get("rollbackError").is_some()
        || before != readback::snapshot(store)?
    {
        return Err(format!(
            "late merge failure lost diagnostics or failed joint rollback: {wire}"
        )
        .into());
    }
    if wire
        .to_string()
        .contains("private synthetic merge history fault")
    {
        return Err("merge diagnostic leaked raw synthetic trigger text".into());
    }
    readback::assert_note(store, note)?;
    Ok(
        json!({"error":wire,"snapshotBefore":before,"snapshotAfter":readback::snapshot(store)?,
        "outerClassificationNalMetadataQueueAndCoreNoteRolledBack":true}),
    )
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let late = late_failure(store, service, key, &note).await?;
    let before = readback::snapshot(store)?;
    let first = merge(store, service, key).await?;
    if first.state != "applied"
        || first.survivor_item_id != fixture::case().survivor
        || first.merged_item_id != fixture::case().source
        || uuid::Uuid::parse_str(&first.id).is_err()
    {
        return Err("first real merge operation differs from its accepted input".into());
    }
    readback::assert_note(store, &note)?;
    readback::placement(store, &fixture::case().survivor)?;
    let metadata = readback::metadata(store)?;
    let after_first = readback::snapshot(store)?;
    service
        .undo_merge_operation(
            store.shared_agent_connection(),
            key.into(),
            first.id.clone(),
        )
        .await?;
    readback::assert_note(store, &note)?;
    readback::placement(store, &fixture::case().source)?;
    let after_undo = readback::snapshot(store)?;
    let (state,decision,archive):(String,Option<String>,Option<String>)=store.connection()?.query_row(
        "select state,(select decision from duplicate_candidates where id=?2),
        (select archived_at from bibliographic_items where id=?3) from merge_operations where id=?1",
        rusqlite::params![first.id,fixture::case().candidate,fixture::case().source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if state != "undone" || decision.is_some() || archive.is_some() {
        return Err("actual undo did not restore candidate/archive/history".into());
    }
    // Undo removes the original deltas, while keeping created concepts,
    // collections and source memberships as specified by the original owner.
    let second = merge(store, service, key).await?;
    if second.id == first.id || second.state != "applied" {
        return Err("explicit second merge reused an operation or failed to settle".into());
    }
    let count: i64 =
        store
            .connection()?
            .query_row("select count(*) from merge_operations", [], |r| r.get(0))?;
    if count != 2 {
        return Err("merge/undo/remerge did not persist exactly two operations".into());
    }
    readback::assert_note(store, &note)?;
    readback::placement(store, &fixture::case().survivor)?;
    let duplicate = absent(store, service, key, &note).await?;
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"lateHistoryRollback":late,
        "firstMerge":first,"secondMerge":second,"metadataAndSemanticQueue":metadata,
        "snapshotBefore":before,"snapshotAfterFirstMerge":after_first,"snapshotAfterUndo":after_undo,
        "sqlSnapshot":readback::snapshot(store)?,"decidedCandidateFailure":duplicate,
        "mergeUndoExplicitRemerge":true,"dynamicT1LegacyCollection":true,
        "coreNoteDerivedProjectionsIncluded":true,"lateWritesRolledBack":true,
        "undoRetainsCreatedConceptsCollectionsAndSourceMemberships":true,
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,
        "snapshotScope":"41 fixed synthetic SQL projections, including metadata/semantic queue and Core Note derived state; not whole-database or file bytes",
        "coreOnly":true}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original_path = directory.join("receipt.json");
    let original_bytes = std::fs::read(&original_path)?;
    let original: Value = serde_json::from_slice(&original_bytes)?;
    if original["status"] != "passed-duplicate-merge-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["noteId"] != fixture::NOTE
        || original["mergeUndoExplicitRemerge"] != true
        || original["lateWritesRolledBack"] != true
    {
        return Err("cold merge requires the original settled synthetic receipt".into());
    }
    let note = &original["noteDocument"];
    readback::assert_note(store, note)?;
    readback::placement(store, &fixture::case().survivor)?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"] {
        return Err("SQL/log cold reopen changed fixed merge/Note projections".into());
    }
    let duplicate = absent(store, service, key, note).await?;
    if before != readback::snapshot(store)? || std::fs::read(original_path)? != original_bytes {
        return Err("cold merge changed data or overwrote its original receipt".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":before,
        "coldDecidedCandidateFailure":duplicate,"originalReceiptUnchanged":true,"coldSqlLogReopen":true,
        "seedOrCommittedMergeReplay":false,"successfulCallsExposeRequestIdentity":false,
        "lateWritesRolledBack":true,"coreOnly":true,"providerCalls":0}),
    )
}
