//! Synthetic loopback HTTP, real Native DOI policy, real Core SQL/log and Note.
use super::*;
use research_workbench_native_sdk::{
    doi_metadata::MetadataPreviewV1,
    import_jobs::{
        ImportJobsRequestV1 as JobRequest, ImportJobsResultV1 as JobResponse, ImportedJob,
    },
    metadata::{
        MetadataCandidateChoice as Choice, MetadataCandidateFieldDecision as Decision,
        MetadataCandidateTrust as Trust, MetadataRequestV1 as Request,
        MetadataResultV1 as Response,
    },
};

#[path = "doi_check/entry.rs"]
mod entry;
#[path = "doi_check/failures.rs"]
mod failures;
#[path = "doi_check/fixture.rs"]
mod fixture;
#[path = "doi_check/readback.rs"]
mod readback;
pub(super) use entry::Mode;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let initial_note = fixture::seed(store)?;
    let mut calls = vec![];
    let (first, note, first_request) = first_lookup(store, service, key).await?;
    if note == initial_note
        || note["blocks"][0]["content"]["items"][0]["text"] != fixture::SAVED_BODY
    {
        return Err("HTTP wait did not allow the real Core Note save".into());
    }
    calls.push(json!({"method":"previewDoi","preview":first,"httpRequest":first_request}));
    let mut draft = first.draft.clone();
    draft.id = Some("doi-cli-caller-id-must-not-create".into());
    draft.source_kind = Some("user".into());
    draft.title = "Confirmed DOI title".into();
    let created = item(
        metadata(
            store,
            service,
            key,
            Request::FullApply {
                candidate_id: first.id.clone(),
                draft: Box::new(draft),
            },
            &mut calls,
        )
        .await?,
    )?;
    if created.id == "doi-cli-caller-id-must-not-create"
        || created.title != "Confirmed DOI title"
        || created.doi != fixture::DOI
        || created.creators.len() != 1
        || created.tags != ["alpha", "beta"]
    {
        return Err("DOI FullApply lost header identity, fields or relationships".into());
    }
    let job = get_job(store, service, key, &first.import_job_id, &mut calls).await?;
    imported(&job, &created.id)?;
    let mut actual = catalog_item(store, service, key, &created.id, &mut calls).await?;
    if actual != created {
        return Err("FullApply and real catalog disagree".into());
    }

    let reviewed = lookup(
        store,
        service,
        key,
        Some(created.id.clone()),
        "Reviewed DOI title",
        "2",
    )
    .await?;
    calls.push(json!({"method":"previewDoi","preview":reviewed}));
    let decisions = reviewed
        .diffs
        .iter()
        .map(|diff| Decision {
            field_name: diff.field_name.clone(),
            choice: if matches!(diff.field_name.as_str(), "title" | "volume") {
                Choice::Candidate
            } else {
                Choice::Current
            },
        })
        .collect();
    actual = item(
        metadata(
            store,
            service,
            key,
            Request::FieldsApply {
                candidate_id: reviewed.id.clone(),
                decisions,
                candidate_trust: Trust::Reviewable,
            },
            &mut calls,
        )
        .await?,
    )?;
    if actual.id != created.id || actual.title != "Reviewed DOI title" || actual.volume != "2" {
        return Err("DOI field review did not update the existing item".into());
    }
    imported(
        &get_job(store, service, key, &reviewed.import_job_id, &mut calls).await?,
        &created.id,
    )?;
    if catalog_item(store, service, key, &created.id, &mut calls).await? != actual {
        return Err("FieldsApply and real catalog disagree".into());
    }

    let rejected = lookup(
        store,
        service,
        key,
        Some(created.id.clone()),
        "Rejected DOI title",
        "3",
    )
    .await?;
    calls.push(json!({"method":"previewDoi","preview":rejected}));
    done(
        metadata(
            store,
            service,
            key,
            Request::Reject {
                candidate_id: rejected.id.clone(),
            },
            &mut calls,
        )
        .await?,
    )?;
    let rejected_job = get_job(store, service, key, &rejected.import_job_id, &mut calls).await?;
    cancelled(&rejected_job)?;
    let cancelled_preview = lookup(
        store,
        service,
        key,
        Some(created.id.clone()),
        "Cancelled DOI title",
        "4",
    )
    .await?;
    calls.push(json!({"method":"previewDoi","preview":cancelled_preview}));
    let response = service
        .apply_import_jobs(
            store.shared_agent_connection(),
            key.into(),
            JobRequest::Cancel {
                id: cancelled_preview.import_job_id.clone(),
            },
        )
        .await?;
    if !matches!(response, JobResponse::Done {}) {
        return Err("DOI cancel returned another DTO".into());
    }
    calls.push(json!({"method":"cancelImportJob","jobId":cancelled_preview.import_job_id,"response":response}));
    let cancel_requested = get_job(
        store,
        service,
        key,
        &cancelled_preview.import_job_id,
        &mut calls,
    )
    .await?;
    if cancel_requested.state != "queued" || !cancel_requested.cancellation_requested {
        return Err("DOI cancellation request changed the original queued-state semantics".into());
    }
    done(
        metadata(
            store,
            service,
            key,
            Request::Reject {
                candidate_id: cancelled_preview.id.clone(),
            },
            &mut calls,
        )
        .await?,
    )?;
    cancelled(
        &get_job(
            store,
            service,
            key,
            &cancelled_preview.import_job_id,
            &mut calls,
        )
        .await?,
    )?;
    if catalog_item(store, service, key, &created.id, &mut calls).await? != actual {
        return Err("Reject/cancel changed the existing literature item".into());
    }
    let failure = failures::run(store, service, key, &mut calls).await?;
    readback::assert_note(store, &note)?;
    let sql = readback::snapshot(store)?;
    if sql["jobs"].as_array().map(Vec::len) != Some(5)
        || sql["results"].as_array().map(Vec::len) != Some(5)
        || sql["candidates"].as_array().map(Vec::len) != Some(4)
        || sql["items"].as_array().map(Vec::len) != Some(1)
    {
        return Err("DOI workflow created an unexpected job/result/candidate/item prefix".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"initialNoteDocument":initial_note,"noteDocument":note,
        "item":actual,"successfulCalls":calls,"resolverFailure":failure,"sqlSnapshot":sql,
        "doiPreviewApplyReviewRejectAndJobs":true,"coreNoteSavedDuringHttpWait":true,
        "syntheticHttpResponseCount":5,"http":"synthetic loopback in the same Tokio process",
        "nativePolicyAndCoreSql":"real selected independent package and production owners",
        "successfulCallsExposeRequestIdentity":false,"providerCalls":0,"coreOnly":true,
        "snapshotScope":"15 explicit SQL projections plus one complete Core Note; not whole database or file bytes"}),
    )
}

async fn first_lookup(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<(MetadataPreviewV1, Value, String), Error> {
    let raw = fixture::response("Original DOI title", "1");
    let fixture::Http {
        url,
        mut seen,
        release,
        task,
    } = fixture::http("200 OK", raw.clone()).await?;
    let future = fixture::preview(
        service,
        store.shared_agent_connection(),
        key.into(),
        fixture::RAW.into(),
        None,
        url,
    );
    tokio::pin!(future);
    tokio::select! {
        reply = &mut future => { let _ = release.send(()); task.abort(); let _ = task.await; reply?; return Err("DOI completed before the synthetic HTTP gate".into()); },
        received = &mut seen => received?,
    }
    let database = store.shared_agent_connection();
    let mut saved = tokio::task::spawn_blocking(move || {
        fixture::save_during_http(&database).map_err(|e| e.to_string())
    });
    let bounded = tokio::time::timeout(std::time::Duration::from_secs(5), &mut saved).await;
    let within_wait = bounded.is_ok();
    let _ = release.send(());
    let preview = future.await;
    let request = task.await??;
    fixture::check_request(&request)?;
    let note = match bounded {
        Ok(result) => result??,
        Err(_) => saved.await??,
    };
    if !within_wait {
        return Err("Core Note save could not finish while HTTP was paused".into());
    }
    let preview = preview?;
    verify_preview(store, &preview, &raw)?;
    Ok((preview, note, request))
}

async fn lookup(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    item_id: Option<String>,
    title: &str,
    volume: &str,
) -> Result<MetadataPreviewV1, Error> {
    let raw = fixture::response(title, volume);
    let fixture::Http {
        url, release, task, ..
    } = fixture::http("200 OK", raw.clone()).await?;
    let _ = release.send(());
    let response = fixture::preview(
        service,
        store.shared_agent_connection(),
        key.into(),
        fixture::RAW.into(),
        item_id,
        url,
    )
    .await;
    fixture::check_request(&task.await??)?;
    let preview = response?;
    verify_preview(store, &preview, &raw)?;
    Ok(preview)
}

fn verify_preview(store: &AppStore, preview: &MetadataPreviewV1, raw: &str) -> Result<(), Error> {
    let sql = readback::snapshot(store)?;
    let row = sql["candidates"]
        .as_array()
        .ok_or("missing candidates")?
        .iter()
        .find(|row| row["id"] == preview.id)
        .ok_or("preview candidate missing")?;
    let draft: research_workbench_native_sdk::catalog_save::CatalogDraft = serde_json::from_str(
        row["draft_json"]
            .as_str()
            .ok_or("candidate draft missing")?,
    )?;
    let audit: Value = serde_json::from_str(
        row["raw_payload"]
            .as_str()
            .ok_or("candidate audit missing")?,
    )?;
    let result = sql["results"]
        .as_array()
        .ok_or("missing results")?
        .iter()
        .find(|result| result["id"] == row["import_result_id"])
        .ok_or("candidate result missing")?;
    if preview.source_reference != fixture::DOI
        || preview.source_kind != "doi.org:csl-json"
        || preview.draft != draft
        || preview.draft.id != preview.item_id
        || !preview.warnings.is_empty()
        || audit["doiResponse"] != raw
        || audit["schemaVersion"] != 1
        || result["job_id"] != preview.import_job_id
        || result["input_label"] != fixture::DOI
        || result["raw_input"] != fixture::RAW
        || result["state"] != "pending"
    {
        return Err(
            "real DOI preview did not preserve normalized/raw input, draft or audit envelope"
                .into(),
        );
    }
    Ok(())
}

async fn metadata(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: Request,
    calls: &mut Vec<Value>,
) -> Result<Response, Error> {
    let response = service
        .apply_metadata(store.shared_agent_connection(), key.into(), request.clone())
        .await?;
    calls.push(json!({"method":"applyMetadata","request":request,"response":response}));
    Ok(response)
}

fn item(response: Response) -> Result<CatalogItem, Error> {
    if let Response::Item { item } = response {
        Ok(*item)
    } else {
        Err("metadata returned Done instead of Item".into())
    }
}
fn done(response: Response) -> Result<(), Error> {
    if matches!(response, Response::Done {}) {
        Ok(())
    } else {
        Err("Reject returned Item instead of Done".into())
    }
}

async fn get_job(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    id: &str,
    calls: &mut Vec<Value>,
) -> Result<ImportedJob, Error> {
    let response = service
        .apply_import_jobs(
            store.shared_agent_connection(),
            key.into(),
            JobRequest::Get { id: id.into() },
        )
        .await?;
    calls.push(json!({"method":"getImportJob","id":id,"response":response}));
    match response {
        JobResponse::Job { job } if job.id == id => Ok(job),
        _ => Err("GetImportJob changed the requested identity".into()),
    }
}

fn imported(job: &ImportedJob, item_id: &str) -> Result<(), Error> {
    if job.source_kind != "doi"
        || job.state != "completed"
        || job.total_count != 1
        || job.completed_count != 1
        || job.results.len() != 1
        || job.results[0].state != "imported"
        || job.results[0].item_id.as_deref() != Some(item_id)
        || job.results[0].raw_input != fixture::RAW
    {
        return Err("DOI confirmation did not update the original job/result".into());
    }
    Ok(())
}
fn cancelled(job: &ImportedJob) -> Result<(), Error> {
    if job.state != "cancelled"
        || !job.cancellation_requested
        || job.results.len() != 1
        || job.results[0].state != "cancelled"
    {
        return Err("DOI Reject did not cancel its original job/result".into());
    }
    Ok(())
}

async fn catalog_item(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    id: &str,
    calls: &mut Vec<Value>,
) -> Result<CatalogItem, Error> {
    let mut items = service
        .catalog(
            store.shared_agent_connection(),
            key.into(),
            CatalogRequest::Get { id: id.into() },
        )
        .await?;
    if items.len() != 1 || items[0].id != id {
        return Err("catalog returned another DOI item".into());
    }
    calls.push(json!({"method":"getCatalogItem","items":items}));
    Ok(items.remove(0))
}

pub(super) async fn reopen(
    store: &AppStore,
    _service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&bytes)?;
    if original["status"] != "passed-doi-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["doiPreviewApplyReviewRejectAndJobs"] != true
        || original["coreNoteSavedDuringHttpWait"] != true
        || original["noteId"] != fixture::NOTE
    {
        return Err("cold DOI check requires its successful original synthetic receipt".into());
    }
    let before = readback::snapshot(store)?;
    let note = fixture::note(store)?;
    if before != original["sqlSnapshot"] || note != original["noteDocument"] {
        return Err("cold SQL/log reopen changed DOI projections or the full Core Note".into());
    }
    readback::assert_note(store, &note)?;
    if readback::snapshot(store)? != before {
        return Err("cold DOI verification changed its projections".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":before,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&bytes)),
        "originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==bytes,
        "coldVerificationNoSeedOrServiceOrHttpOrNativeStart":true,"coreOnly":true,
        "snapshotScope":original["snapshotScope"]}),
    )
}
