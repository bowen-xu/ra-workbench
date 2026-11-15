//! Real independent Native preparation and Core reference SQL/log/Note owners.
use super::*;
use crate::runtime_plugins::literature_reference_records::{
    ReferenceOutcome, ReferenceRequest as Request, ReferenceResult as Response,
};
use research_workbench_native_sdk::reference_records::{ReferenceRecord, ReferenceRecordDraft};

#[path = "reference_records_check/entry.rs"]
mod entry;
#[path = "reference_records_check/fixture.rs"]
mod fixture;
#[path = "reference_records_check/readback.rs"]
mod readback;
pub(super) use entry::Mode;

async fn list(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    paper_id: &str,
) -> Result<Vec<ReferenceRecord>, Error> {
    match service
        .reference_records(
            store.shared_agent_connection(),
            key.into(),
            Request::List {
                paper_id: paper_id.into(),
            },
        )
        .await?
    {
        Response::Records(records) => Ok(records),
        Response::Record(_) => Err("reference list returned another result kind".into()),
    }
}

async fn save(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    draft: ReferenceRecordDraft,
) -> Result<ReferenceRecord, Error> {
    match service
        .reference_records(
            store.shared_agent_connection(),
            key.into(),
            Request::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        Response::Record(record) => Ok(*record),
        Response::Records(_) => Err("reference save returned another result kind".into()),
    }
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    if !list(store, service, key, fixture::CITING).await?.is_empty() {
        return Err("new synthetic reference list was not empty".into());
    }
    let create_draft = fixture::draft();
    let created = save(store, service, key, create_draft.clone()).await?;
    uuid::Uuid::parse_str(&created.id)?;
    if created.revision != 1
        || created.citing_paper_id != fixture::CITING
        || created.source_anchor_id != fixture::SOURCE
        || created.raw_text != create_draft.raw_text.trim()
        || created.fields != create_draft.fields
        || created.parse_state != "parsed"
        || created.identity_state != "unverified"
        || created.matched_item_id.is_some()
        || created.semantic_state != "unreviewed"
        || created.usage_kind.is_some()
        || !created.usage_summary.is_empty()
        || created.usage_anchor_id.is_some()
        || created.provenance_kind != "human_original"
        || created.created_at.is_empty()
        || created.updated_at.is_empty()
    {
        return Err("real reference create changed the original DTO semantics".into());
    }
    let mut update_draft = create_draft.clone();
    update_draft.id = Some(created.id.clone());
    update_draft.expected_revision = Some(1);
    update_draft.identity_state = "matched_existing".into();
    update_draft.matched_item_id = Some(fixture::CITED.into());
    update_draft.semantic_state = "reviewed".into();
    update_draft.usage_kind = Some("supports".into());
    update_draft.usage_summary = "  Synthetic reviewed use  ".into();
    update_draft.usage_anchor_id = Some(fixture::USAGE.into());
    update_draft.fields[0].value = "  Revised cited title  ".into();
    update_draft.change_reason = " manual_reference_verification ".into();
    let updated = save(store, service, key, update_draft.clone()).await?;
    if updated.id != created.id
        || updated.revision != 2
        || updated.created_at != created.created_at
        || updated.raw_text != created.raw_text
        || updated.fields != update_draft.fields
        || updated.parse_state != created.parse_state
        || updated.citing_paper_id != created.citing_paper_id
        || updated.source_anchor_id != created.source_anchor_id
        || updated.identity_state != "matched_existing"
        || updated.matched_item_id.as_deref() != Some(fixture::CITED)
        || updated.semantic_state != "reviewed"
        || updated.usage_kind.as_deref() != Some("supports")
        || updated.usage_summary != "Synthetic reviewed use"
        || updated.usage_anchor_id.as_deref() != Some(fixture::USAGE)
        || updated.provenance_kind != created.provenance_kind
        || updated.updated_at.is_empty()
    {
        return Err(
            "real reference update lost identity, revision or independent review states".into(),
        );
    }
    let before_conflict = readback::snapshot(store)?;
    let mut stale = update_draft.clone();
    stale.raw_text = "private synthetic stale reference draft".into();
    let error = match service
        .reference_records(
            store.shared_agent_connection(),
            key.into(),
            Request::Save {
                draft: Box::new(stale),
            },
        )
        .await
    {
        Err(error) => error,
        Ok(_) => return Err("stale reference version unexpectedly committed".into()),
    };
    let failure = error
        .reference_records
        .as_ref()
        .ok_or("reference conflict lost typed progress")?;
    if error.code != "revision_conflict"
        || error.message != "参考条目版本冲突；当前版本为 v2"
        || error.current_revision != Some(2)
        || failure.progress.current_revision != Some(2)
        || failure.progress.record_id.as_deref() != Some(updated.id.as_str())
        || failure.progress.phase != "revision"
        || failure.progress.outcome != ReferenceOutcome::RolledBack
        || failure.rollback_error.is_some()
        || error.cleanup_error.is_some()
        || error.package_key.as_deref() != Some(key)
    {
        return Err(
            "reference conflict changed its actual version, rollback or package identity".into(),
        );
    }
    uuid::Uuid::parse_str(
        error
            .instance_id
            .as_deref()
            .ok_or("reference conflict lost instanceId")?,
    )?;
    uuid::Uuid::parse_str(
        error
            .request_id
            .as_deref()
            .ok_or("reference conflict lost requestId")?,
    )?;
    let diagnostic = serde_json::to_value(&error)?;
    if serde_json::to_string(&diagnostic)?.contains("private synthetic stale reference draft") {
        return Err("reference conflict exposed a private synthetic draft".into());
    }
    if readback::snapshot(store)? != before_conflict {
        return Err("reference conflict changed the original committed SQL prefix".into());
    }
    let final_records = list(store, service, key, fixture::CITING).await?;
    if final_records != vec![updated.clone()]
        || !list(store, service, key, fixture::CITED).await?.is_empty()
    {
        return Err("reference list changed record identity or citing-paper ownership".into());
    }
    let final_sql = readback::snapshot(store)?;
    readback::history(&final_sql, &updated.id, &[create_draft, update_draft])?;
    if readback::protected(&initial)? != readback::protected(&final_sql)?
        || fixture::note(store)? != note
        || !store.connection()?.is_autocommit()
    {
        return Err(
            "reference workflow changed Core Note/anchor owners or left a transaction open".into(),
        );
    }
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"initialSql":initial,
        "createdRecord":created,"savedRecord":updated,"finalRecords":final_records,
        "conflict":diagnostic,"sqlSnapshot":final_sql,"successfulCalls":5,"conflicts":1,
        "createUpdateConflictList":true,"immutableRevisionCount":2,
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,"coreOnly":true,
        "methodScope":"ReferenceRecordGateway.list/save",
        "snapshotScope":"9 explicit synthetic SQL projections plus full Core Note document; not whole database or database-file bytes"
    }))
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original_bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&original_bytes)?;
    if original["status"] != "passed-reference-records-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["noteId"] != fixture::NOTE
        || original["methodScope"] != "ReferenceRecordGateway.list/save"
        || original["createUpdateConflictList"] != true
        || original["immutableRevisionCount"] != 2
    {
        return Err(
            "cold reference check requires its successful original synthetic receipt".into(),
        );
    }
    let actual = readback::snapshot(store)?;
    let note = fixture::note(store)?;
    if actual != original["sqlSnapshot"] || note != original["noteDocument"] {
        return Err(
            "fresh SQL/log reopen changed reference identity/history, anchor or full Core Note"
                .into(),
        );
    }
    let saved: ReferenceRecord = serde_json::from_value(original["savedRecord"].clone())?;
    if actual["records"][0]["id"] != saved.id
        || actual["records"][0]["current_revision"] != saved.revision
        || saved.revision != 2
        || actual["revisions"].as_array().map(Vec::len) != Some(2)
        || original["finalRecords"] != json!([saved])
    {
        return Err("cold reference SQL identity disagrees with the original returned DTO".into());
    }
    if readback::snapshot(store)? != actual || !store.connection()?.is_autocommit() {
        return Err("cold reference verification changed a fixed projection".into());
    }
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":actual,
        "originalReturnedRecords":original["finalRecords"],
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&original_bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==original_bytes,
        "coldReopenNoSeedOrRequestReplay":true,"noReferenceServiceCallDuringReopen":true,
        "noNativeRequestDuringReopen":true,"coreOnly":true,
        "methodScope":original["methodScope"],"snapshotScope":original["snapshotScope"]
    }))
}
