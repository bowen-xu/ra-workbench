//! Real external-host undo calls; all data belongs to this synthetic Core owner.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "duplicate_undo_check/fixture.rs"]
mod fixture;
#[path = "duplicate_undo_check/readback.rs"]
mod readback;

const CONFLICT: &str = "合并后的分类已修改，请先恢复分类到合并后的状态再撤销合并";

struct ExpectedFailure {
    code: &'static str,
    outcome: &'static str,
    phase: &'static str,
}

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    operation: &str,
    expected: ExpectedFailure,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let error = service
        .undo_merge_operation(
            store.shared_agent_connection(),
            key.into(),
            operation.into(),
        )
        .await
        .err()
        .ok_or("synthetic undo failure unexpectedly succeeded")?;
    readback::assert_note(store, note)?;
    let after = readback::snapshot(store)?;
    let wire = serde_json::to_value(&error)?;
    if error.code != expected.code
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || wire["operationId"] != operation
        || error.instance_id.is_none()
        || wire["duplicateUndo"]["outcome"] != expected.outcome
        || wire["duplicateUndo"]["problem"]["phase"] != expected.phase
        || wire["duplicateUndo"].get("rollbackError").is_some()
        || before != after
    {
        return Err(format!("undo failure lost identity/outcome/phase or did not preserve its fixed snapshot: {wire}").into());
    }
    if expected.outcome == "rolledBack" && error.request_id.is_none() {
        return Err("dispatched undo failure lost its native request identity".into());
    }
    if expected.code == "invalid_input" && error.message != CONFLICT {
        return Err("undo after-conflict original message changed".into());
    }
    if expected.code == "connector_error" && !error.message.starts_with("DOI 元数据请求失败：")
    {
        return Err("original stored-snapshot codec error prefix changed".into());
    }
    if wire.to_string().contains("private synthetic undo fault") {
        return Err("undo SQL diagnostic exposed raw trigger text".into());
    }
    Ok(json!({"operationId":operation,"error":wire,"fixedSnapshotUnchanged":true}))
}

fn conflict_fixture(store: &AppStore, label: &str) -> Result<fixture::Case, Error> {
    let mut case = fixture::seed(store, label, fixture::Mode::Full)?;
    case.snapshot
        .classification
        .as_mut()
        .ok_or("classification missing")?
        .after_memberships
        .pop();
    fixture::change_snapshot(store, &case, &serde_json::to_string(&case.snapshot)?)?;
    Ok(case)
}

async fn late_failures(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Vec<Value>, Error> {
    let case = fixture::seed(store, "late-mark", fixture::Mode::Full)?;
    store.connection()?.execute_batch(
        "create temp trigger synthetic_undo_mark_fail before update of state on merge_operations
         when old.id='undo-late-mark' and new.state='undone' begin
           select raise(abort,'private synthetic undo fault'); end;",
    )?;
    let mark = rejected(
        store,
        service,
        key,
        note,
        &case.id,
        ExpectedFailure {
            code: "storage_error",
            outcome: "rolledBack",
            phase: "restore",
        },
    )
    .await;
    store
        .connection()?
        .execute_batch("drop trigger synthetic_undo_mark_fail")?;
    let mark = mark?;
    if mark["error"]["duplicateUndo"]["problem"]["cause"]["diagnostic"]["sqliteExtendedCode"]
        != 1811
    {
        return Err("late markUndone failure lost its safe SQLite 1811 cause".into());
    }

    let case = fixture::seed(store, "late-search", fixture::Mode::Full)?;
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    store
        .connection()?
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "literature_search"
                }
            ) {
                observed.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let search = rejected(
        store,
        service,
        key,
        note,
        &case.id,
        ExpectedFailure {
            code: "storage_error",
            outcome: "rolledBack",
            phase: "search",
        },
    )
    .await;
    store
        .connection()?
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let search = search?;
    if attempts.load(Ordering::SeqCst) == 0
        || search["error"]["duplicateUndo"]["problem"]["cause"]["diagnostic"]["sqliteExtendedCode"]
            != 23
    {
        return Err("actual late FTS write was not rejected with safe SQLite AUTH cause".into());
    }
    Ok(vec![
        mark,
        json!({"failure":search,"ftsInsertAttempts":attempts.load(Ordering::SeqCst)}),
    ])
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed_note(store)?;
    let mut successes = vec![];
    for (label, mode) in [
        ("full", fixture::Mode::Full),
        ("none", fixture::Mode::NoClassification),
        ("nal-none", fixture::Mode::NoNal),
    ] {
        let case = fixture::seed(store, label, mode)?;
        let before = readback::snapshot(store)?;
        service
            .undo_merge_operation(store.shared_agent_connection(), key.into(), case.id.clone())
            .await?;
        readback::assert_note(store, &note)?;
        let after = readback::assert_success(store, &case, &before)?;
        successes.push(json!({"operationId":case.id,"mode":label,"snapshotBefore":before,"snapshotAfter":after}));
    }
    let duplicate = rejected(
        store,
        service,
        key,
        &note,
        "undo-full",
        ExpectedFailure {
            code: "not_found",
            outcome: "notStarted",
            phase: "header",
        },
    )
    .await?;
    let missing = rejected(
        store,
        service,
        key,
        &note,
        "undo-missing",
        ExpectedFailure {
            code: "not_found",
            outcome: "notStarted",
            phase: "header",
        },
    )
    .await?;
    let case = fixture::seed(store, "bad-snapshot", fixture::Mode::Full)?;
    fixture::change_snapshot(store, &case, "{\"createdReadingState\":\"wrong\"}")?;
    let invalid = rejected(
        store,
        service,
        key,
        &note,
        &case.id,
        ExpectedFailure {
            code: "connector_error",
            outcome: "notStarted",
            phase: "snapshot",
        },
    )
    .await?;
    let late = late_failures(store, service, key, &note).await?;
    // This applied operation remains conflicting. Cold verification can issue
    // another real Native prepare without replaying a committed restoration.
    let conflict = conflict_fixture(store, "conflict")?;
    let rejected_conflict = rejected(
        store,
        service,
        key,
        &note,
        &conflict.id,
        ExpectedFailure {
            code: "invalid_input",
            outcome: "rolledBack",
            phase: "classificationCheck",
        },
    )
    .await?;
    readback::assert_note(store, &note)?;
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"successes":successes,
        "duplicateAppliedFailure":duplicate,"missingFailure":missing,"snapshotFailure":invalid,
        "lateRollbackFailures":late,"conflictFailure":rejected_conflict,
        "coldConflictOperation":conflict.id,"sqlSnapshot":readback::snapshot(store)?,
        "optionalBranchesPreserved":true,"noncanonicalStoredFactDecoded":true,
        "coreNoteDerivedProjectionsIncluded":true,"lateWritesRolledBack":true,
        "successfulCallsExposeRequestIdentity":false,
        "snapshotScope":"29 fixed projections of synthetic undo rows, two state revisions, literature FTS and one Core note/document/catalog/metadata/search; not whole-database or file bytes",
        "coreOnly":true
    }))
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
    if original["status"] != "passed-duplicate-undo-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["noteId"] != fixture::NOTE
        || original["optionalBranchesPreserved"] != true
        || original["lateWritesRolledBack"] != true
    {
        return Err(
            "cold undo requires the original successfully settled synthetic receipt".into(),
        );
    }
    let note = &original["noteDocument"];
    readback::assert_note(store, note)?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"] {
        return Err("SQL/log cold reopen changed the fixed undo/Core note projections".into());
    }
    let operation = original["coldConflictOperation"]
        .as_str()
        .ok_or("cold conflict identity missing")?;
    let failure = rejected(
        store,
        service,
        key,
        note,
        operation,
        ExpectedFailure {
            code: "invalid_input",
            outcome: "rolledBack",
            phase: "classificationCheck",
        },
    )
    .await?;
    if readback::snapshot(store)? != before || std::fs::read(original_path)? != original_bytes {
        return Err("cold undo changed data or overwrote the original successful receipt".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":before,
        "coldConflictFailure":failure,"originalReceiptUnchanged":true,"coldSqlLogReopen":true,
        "seedOrCommittedUndoReplay":false,"optionalBranchesPreserved":true,
        "lateWritesRolledBack":true,"coreOnly":true}),
    )
}
