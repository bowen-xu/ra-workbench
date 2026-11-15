//! Real six-entry lifecycle calls through the selected independent native host.
use super::*;
use research_workbench_native_sdk::literature_lifecycle::LifecycleRequestV1 as Request;

#[path = "lifecycle_check/failures.rs"]
mod failures;
#[path = "lifecycle_check/fixture.rs"]
mod fixture;
#[path = "lifecycle_check/readback.rs"]
mod readback;

async fn applied(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: Request,
) -> Result<Value, Error> {
    service
        .apply_literature_lifecycle(store.shared_agent_connection(), key.into(), request.clone())
        .await?;
    Ok(json!({"request":request,"sqlSnapshot":readback::snapshot(store)?}))
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    let mut successes = vec![];
    for id in [fixture::MAIN, fixture::NUL_ITEM] {
        for (request, archived, trashed) in [
            (Request::Archive { item_id: id.into() }, true, false),
            (Request::Restore { item_id: id.into() }, false, false),
            (Request::Trash { item_id: id.into() }, false, true),
            (Request::RestoreTrash { item_id: id.into() }, false, false),
        ] {
            successes.push(applied(store, service, key, request).await?);
            readback::item(store, id, archived, trashed)?;
            readback::assert_note(store, &note)?;
        }
    }
    successes.push(
        applied(
            store,
            service,
            key,
            Request::RestoreTrash {
                item_id: "lifecycle-mixed".into(),
            },
        )
        .await?,
    );
    readback::item(store, "lifecycle-mixed", false, false)?;
    let projects = vec![
        "lifecycle-project-a".into(),
        "lifecycle-project-b".into(),
        fixture::NUL_PROJECT.into(),
    ];
    successes.push(
        applied(
            store,
            service,
            key,
            Request::SetProjects {
                item_id: fixture::MAIN.into(),
                project_ids: projects.clone(),
            },
        )
        .await?,
    );
    let mut expected = projects;
    expected.sort();
    if readback::project_ids(store)? != expected {
        return Err("SetProjects lost original/NUL project identities".into());
    }
    // Empty assignment for an absent item is the original zero-write success.
    successes.push(
        applied(
            store,
            service,
            key,
            Request::SetProjects {
                item_id: "lifecycle-missing".into(),
                project_ids: vec![],
            },
        )
        .await?,
    );
    let failures = failures::run(store, service, key).await?;
    successes.push(
        applied(
            store,
            service,
            key,
            Request::TrashMany {
                item_ids: vec![
                    "lifecycle-batch-b".into(),
                    "lifecycle-batch-a".into(),
                    "lifecycle-batch-a".into(),
                ],
            },
        )
        .await?,
    );
    for id in ["lifecycle-batch-a", "lifecycle-batch-b"] {
        readback::item(store, id, false, true)?;
    }
    readback::item(store, "lifecycle-batch-c", false, false)?;
    readback::assert_note(store, &note)?;
    let final_sql = readback::snapshot(store)?;
    if readback::protected(&initial)? != readback::protected(&final_sql)? {
        return Err(
            "lifecycle changed preserved attachments, anchors, tags, search or Core note relations"
                .into(),
        );
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"initialSql":initial,
        "successes":successes,"failures":failures,"sqlSnapshot":final_sql,
        "sixEntrypoints":true,"nulIdentityPreserved":true,"nullableStatePreserved":true,
        "batchFirstErrorRollback":true,"projectDuplicateAndForeignKeyRollback":true,
        "singleFailEffectsPersisted":true,"successfulCallsExposeRequestIdentity":false,
        "providerCalls":0,"coreOnly":true,
        "snapshotScope":"15 explicit synthetic SQL projections plus full Core Note document; not whole database or database-file bytes"}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original_bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&original_bytes)?;
    if original["status"] != "passed-lifecycle-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["noteId"] != fixture::NOTE
        || original["sixEntrypoints"] != true
        || original["singleFailEffectsPersisted"] != true
    {
        return Err(
            "cold lifecycle check requires its successful original synthetic receipt".into(),
        );
    }
    let before = readback::snapshot(store)?;
    let note = fixture::note(store)?;
    if before != original["sqlSnapshot"] || note != original["noteDocument"] {
        return Err(
            "fresh SQL/log reopen changed lifecycle state, trigger effect, relations or Core Note"
                .into(),
        );
    }
    readback::item(store, fixture::NUL_ITEM, false, false)?;
    readback::item(store, fixture::FAIL, false, false)?;
    readback::assert_note(store, &note)?;
    if readback::snapshot(store)? != before {
        return Err("cold lifecycle verification wrote to its fixed projections".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":before,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&original_bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==original_bytes,
        "coldReopenNoSeedOrRequestReplay":true,"noLifecycleCallDuringReopen":true,
        "coreOnly":true,"snapshotScope":original["snapshotScope"]}),
    )
}
