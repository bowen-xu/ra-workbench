//! Real expiry cleanup and trash reads through the selected finite native host.
use super::*;

#[path = "trash_cleanup_check/fixture.rs"]
mod fixture;
#[path = "trash_cleanup_check/readback.rs"]
mod readback;

pub(super) fn artifacts(
    catalog: &RuntimePluginCatalog,
    package: &crate::runtime_plugins::RuntimePluginPackage,
) -> Result<Value, Error> {
    let entry = package.manifest["native"]["entry"]
        .as_str()
        .ok_or("cleanup package has no native entry")?;
    let native = catalog.resource(&package.package_key, entry)?;
    let manifest = catalog.resource(&package.package_key, "manifest.json")?;
    let core = std::fs::read(std::env::current_exe()?)?;
    Ok(
        json!({"coreExecutableSha256":format!("{:x}",Sha256::digest(&core)),
        "coreExecutableBytes":core.len(),"nativeEntry":entry,
        "nativeExecutableSha256":format!("{:x}",Sha256::digest(&native.bytes)),
        "nativeExecutableBytes":native.bytes.len(),
        "manifestSha256":format!("{:x}",Sha256::digest(&manifest.bytes)),
        "artifactScope":"exact current Core executable and selected catalog snapshot resources; no OS PID enumeration"}),
    )
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let notes = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    let error = service
        .purge_expired_trash(store.shared_agent_connection(), key.into())
        .await
        .err()
        .ok_or("synthetic second-item cleanup should fail")?;
    let failure = serde_json::to_value(&error)?;
    if error.code != "storage_error"
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || error.operation_id.is_some()
        || error.cleanup_error.is_some()
        || error.exit_code.is_some()
        || error.signal.is_some()
        || failure["trashCleanup"]["phase"] != "item-delete"
        || failure["trashCleanup"]["outcome"]["state"] != "rolledBack"
        || failure["trashCleanup"]["itemId"] != fixture::SECOND
        || failure["trashCleanup"]["committedItemIds"] != json!([fixture::FIRST])
        || error
            .data_error
            .as_ref()
            .and_then(|cause| cause.sqlite_extended_code)
            != Some(1811)
    {
        return Err(format!("cleanup lost its actual partial-commit diagnostic: {failure}").into());
    }
    for identity in [error.instance_id.as_deref(), error.request_id.as_deref()] {
        if identity.is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("cleanup failure lost actual host request/instance identity".into());
        }
    }
    if failure.to_string().contains(fixture::PRIVATE) || failure.to_string().contains(fixture::BODY)
    {
        return Err("cleanup diagnostic exposed synthetic private text".into());
    }
    let after_failure = readback::snapshot(store)?;
    readback::assert_partial(store, &notes)?;
    readback::assert_second_rollback(&initial, &after_failure)?;
    let mut lists = vec![];
    for (query, expected) in [
        (None, vec![fixture::RECENT, fixture::THIRD, fixture::SECOND]),
        (
            Some(" \t "),
            vec![fixture::RECENT, fixture::THIRD, fixture::SECOND],
        ),
        (Some("needle"), vec![fixture::SECOND]),
        (Some("2024"), vec![fixture::THIRD]),
        (Some("absent-cleanup-token"), vec![]),
    ] {
        let items = service
            .list_trashed_catalog(
                store.shared_agent_connection(),
                key.into(),
                query.map(str::to_owned),
            )
            .await?;
        if ids(&items) != expected {
            return Err(format!("trash query {query:?} lost original visibility/order").into());
        }
        lists.push(json!({"query":query,"items":items}));
        if readback::snapshot(store)? != after_failure {
            return Err("list-after-failed-purge changed the fixed SQL projections".into());
        }
        readback::assert_partial(store, &notes)?;
    }
    Ok(
        json!({"noteIds":[fixture::NOTE,fixture::READING_NOTE],"noteDocuments":notes,
        "initialSql":initial,"partialFailure":failure,"trashLists":lists,
        "sqlSnapshot":after_failure,"firstItemCommitted":true,"secondItemRolledBack":true,
        "thirdItemNotAttempted":true,"listContinuedAfterPurgeFailure":true,
        "sharedContentAndIndependentDataPreserved":true,"coreNoteDerivedRowsPreserved":true,
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,"coreOnly":true,
        "snapshotScope":"25 fixed explicit synthetic SQL projections and two full Core Note documents; not whole database or database-file bytes"}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&bytes)?;
    if original["status"] != "passed-trash-cleanup-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["firstItemCommitted"] != true
        || original["secondItemRolledBack"] != true
        || original["listContinuedAfterPurgeFailure"] != true
        || original["noteIds"] != json!([fixture::NOTE, fixture::READING_NOTE])
    {
        return Err("cold cleanup check requires its successful original synthetic receipt".into());
    }
    let before = readback::snapshot(store)?;
    let notes = fixture::notes(store)?;
    if before != original["sqlSnapshot"] || notes != original["noteDocuments"] {
        return Err("cold SQL/log reopen changed cleanup state or retained Core Notes".into());
    }
    readback::assert_partial(store, &notes)?;
    if readback::snapshot(store)? != before {
        return Err("cold cleanup verification wrote to its fixed projections".into());
    }
    Ok(
        json!({"noteIds":original["noteIds"],"noteDocuments":notes,"sqlSnapshot":before,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==bytes,
        "coldReopenNoSeedOrRequestReplay":true,"noPurgeOrListCallDuringReopen":true,
        "temporaryFaultTriggerNotRecreated":true,"coreOnly":true,
        "snapshotScope":original["snapshotScope"]}),
    )
}
