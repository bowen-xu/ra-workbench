//! Four original import consumers through the selected real finite native host.
use super::*;
use research_workbench_native_sdk::{
    catalog_save::CatalogDraft,
    import_jobs::{ImportJobsRequestV1 as Request, ImportJobsResultV1 as Response, ImportedJob},
};

#[path = "import_jobs_check/entry.rs"]
mod entry;
#[path = "import_jobs_check/failures.rs"]
mod failures;
#[path = "import_jobs_check/fixture.rs"]
mod fixture;
#[path = "import_jobs_check/readback.rs"]
mod readback;
#[path = "import_jobs_check/scenarios.rs"]
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
        .apply_import_jobs(store.shared_agent_connection(), key.into(), request.clone())
        .await?;
    readback::assert_note(store, note)?;
    calls.push(json!({"request":request,"response":response}));
    Ok(response)
}

fn job(response: Response) -> Result<ImportedJob, Error> {
    match response {
        Response::Job { job } => Ok(job),
        _ => Err("import request returned a different result kind".into()),
    }
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    let mut calls = vec![];
    let mixed = scenarios::run(store, service, key, &note, &mut calls).await?;
    let errors = failures::run(store, service, key, &note, &mut calls).await?;
    readback::assert_note(store, &note)?;
    let final_sql = readback::snapshot(store)?;
    for name in ["notePages", "noteRevisions", "noteLinks", "candidates"] {
        if initial[name] != final_sql[name] {
            return Err("import jobs changed unrelated Core Note or candidate storage".into());
        }
    }
    let independent = |value: &Value| -> Result<Value, Error> {
        value["items"]
            .as_array()
            .ok_or("missing item snapshot")?
            .iter()
            .find(|row| row["id"] == fixture::INDEPENDENT)
            .cloned()
            .ok_or_else(|| "independent import material missing".into())
    };
    if independent(&initial)? != independent(&final_sql)? {
        return Err("import batch changed independent material".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"initialSql":initial,
        "successfulCalls":calls,"mixedBatch":mixed,"rejectedCalls":errors,"sqlSnapshot":final_sql,
        "fourImportConsumers":true,"rawDraftSourceAndLabelsPreserved":true,"partialCommitsPreserved":true,
        "sharedDoiPdfJobViewsAndCancellation":true,"originalBusinessSqlFailurePreserved":true,
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,"coreOnly":true,
        "snapshotScope":"16 explicit SQL projections with all selected timestamps and f64 bits plus one full independent Core Note document; not whole database or database-file bytes",
        "jobFixtureScope":"DOI/PDF jobs are synthetic queue records only. No DOI fetch, PDF file import or worker is scheduled. Queued/running recovery is verified on cold reopen."}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
    recovery_start: Option<&str>,
) -> Result<Value, Error> {
    let start = recovery_start.ok_or("cold import check lacks its pre-open SQLite clock")?;
    let end = fixture::clock(store)?;
    if start > end.as_str() {
        return Err("SQLite recovery clock interval reversed".into());
    }
    let bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&bytes)?;
    if original["status"] != "passed-import-jobs-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["fourImportConsumers"] != true
        || original["partialCommitsPreserved"] != true
        || original["noteId"] != fixture::NOTE
    {
        return Err("cold import check requires its successful original synthetic receipt".into());
    }
    let actual = readback::snapshot(store)?;
    let mut expected = original["sqlSnapshot"].clone();
    let old_jobs = expected["jobs"]
        .as_array_mut()
        .ok_or("missing original jobs")?;
    let current_jobs = actual["jobs"].as_array().ok_or("missing reopened jobs")?;
    if old_jobs.len() != current_jobs.len() {
        return Err("cold import recovery changed job identities".into());
    }
    let mut recovered = vec![];
    for (old, current) in old_jobs.iter_mut().zip(current_jobs) {
        if old["id"] != current["id"] {
            return Err("cold import recovery reordered job identities".into());
        }
        if matches!(old["state"].as_str(), Some("queued" | "running")) {
            let time = current["updated_at"]
                .as_str()
                .ok_or("missing recovery timestamp")?;
            if current["state"] != "interrupted" || time < start || time > end.as_str() {
                return Err(
                    "original startup recovery state/time is outside its SQLite interval".into(),
                );
            }
            recovered.push(old["id"].clone());
            old["state"] = json!("interrupted");
            old["updated_at"] = current["updated_at"].clone();
        }
        if old != current {
            return Err("cold import recovery changed an additional job field".into());
        }
    }
    if recovered.len() != 4
        || actual != expected
        || fixture::note(store)? != original["noteDocument"]
    {
        return Err(
            "cold SQL/log import recovery changed fixed data, raw results or Core Note".into(),
        );
    }
    readback::assert_note(store, &original["noteDocument"])?;
    if readback::snapshot(store)? != actual {
        return Err("cold import verification changed fixed projections".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":original["noteDocument"],"sqlSnapshot":actual,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==bytes,
        "recoverySqliteClock":{"start":start,"end":end},"recoveredJobIds":recovered,
        "onlyOriginalQueuedRunningRecoveryCellsChanged":true,"coldReopenNoSeedOrRequestReplay":true,
        "noImportServiceDispatchDuringReopen":true,"temporaryFaultTriggersNotRecreated":true,
        "coreOnly":true,"snapshotScope":original["snapshotScope"]}),
    )
}
