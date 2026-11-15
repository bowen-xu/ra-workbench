//! Five metadata consumers through the selected real native service and Core DB.
use super::*;
use research_workbench_native_sdk::{
    catalog::{CatalogCreator, CatalogItem},
    catalog_save::CatalogDraft,
    metadata::{
        LiteratureMetadataPatch as Patch, MetadataCandidateChoice as Choice,
        MetadataCandidateFieldDecision as Decision, MetadataCandidateTrust as Trust,
        MetadataRequestV1 as Request, MetadataResultV1 as Response,
    },
};

#[path = "metadata_check/entry.rs"]
mod entry;
#[path = "metadata_check/failures.rs"]
mod failures;
#[path = "metadata_check/fixture.rs"]
mod fixture;
#[path = "metadata_check/readback.rs"]
mod readback;
#[path = "metadata_check/scenarios.rs"]
mod scenarios;
pub(super) use entry::Mode;

async fn apply(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    request: Request,
    calls: &mut Vec<Value>,
) -> Result<Response, Error> {
    let response = service
        .apply_metadata(store.shared_agent_connection(), key.into(), request.clone())
        .await?;
    readback::assert_note(store, note)?;
    calls.push(
        json!({"request":request,"response":response,"sqlSnapshot":readback::snapshot(store)?}),
    );
    Ok(response)
}

fn item(response: Response) -> Result<CatalogItem, Error> {
    match response {
        Response::Item { item } => Ok(*item),
        Response::Done {} => Err("metadata item request returned Done".into()),
    }
}

fn draft_from_item(item: &CatalogItem) -> Result<CatalogDraft, Error> {
    // Mechanical fixture copy of the public fields, not a policy implementation.
    Ok(serde_json::from_value(serde_json::to_value(item)?)?)
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    let mut calls = Vec::new();
    let verified = scenarios::run(store, service, key, &note, &mut calls).await?;
    let rejected = failures::run(store, service, key, &note).await?;
    readback::assert_note(store, &note)?;
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"initialSql":initial,
        "successfulCalls":calls,"verified":verified,"rejectedCalls":rejected,
        "successfulCallCount":calls.len(),"rejectedCallCount":rejected.len(),
        "sqlSnapshot":readback::snapshot(store)?,"fiveMetadataConsumers":true,
        "hiddenFieldsProjectionNotFoundBound":true,
        "duplicateTagsOriginalConstraintRolledBack":true,
        "fullSecondTransactionFailureKeepsFirstSave":true,
        "rejectFailPreservesPriorAutocommitAndFailEffects":true,
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,"coreOnly":true,
        "snapshotScope":"16 explicit SQL projections with actual timestamps/f64 bits, plus one complete independent Core Note document; not whole database or file bytes",
        "jobFixtureScope":"Synthetic import jobs start interrupted without scheduling; final states are completed/cancelled/interrupted, not queued/running recovery acceptance"
    }))
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&bytes)?;
    if original["status"] != "passed-metadata-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["fiveMetadataConsumers"] != true
        || original["hiddenFieldsProjectionNotFoundBound"] != true
        || original["duplicateTagsOriginalConstraintRolledBack"] != true
        || original["rejectedCallCount"] != 7
        || original["fullSecondTransactionFailureKeepsFirstSave"] != true
        || original["rejectFailPreservesPriorAutocommitAndFailEffects"] != true
        || original["noteId"] != fixture::NOTE
    {
        return Err(
            "cold metadata check requires its successful original synthetic receipt".into(),
        );
    }
    let before = readback::snapshot(store)?;
    let note = fixture::note(store)?;
    if before != original["sqlSnapshot"] || note != original["noteDocument"] {
        return Err("cold SQL/log reopen changed metadata projections or Core Note".into());
    }
    readback::assert_note(store, &note)?;
    if readback::snapshot(store)? != before {
        return Err("cold metadata verification changed its fixed SQL projections".into());
    }
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":before,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==bytes,
        "coldReopenNoSeedOrRequestReplay":true,"noMetadataCallOrNativeStartDuringReopen":true,
        "temporaryFaultTriggersNotRecreated":true,"coreOnly":true,
        "snapshotScope":original["snapshotScope"]
    }))
}
