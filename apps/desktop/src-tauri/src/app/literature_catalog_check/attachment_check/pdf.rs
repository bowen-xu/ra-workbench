use super::scenarios::{diagnostic, require};
use super::*;
use std::time::Duration;

async fn get_job(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    id: &str,
) -> Result<ImportedJob, Error> {
    match service
        .apply_import_jobs(
            store.shared_agent_connection(),
            key.into(),
            ImportJobsRequestV1::Get { id: id.into() },
        )
        .await?
    {
        ImportJobsResultV1::Job { job } => Ok(job),
        _ => Err("PDF job result kind".into()),
    }
}
pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let bytes = fixture::pdf("Synthetic background PDF");
    let source = fixture::source(directory, "background.pdf", &bytes)?;
    let candidates = vec![
        PdfImportCandidate {
            path: source.to_str().ok_or("PDF path")?.into(),
            draft: fixture::draft("Synthetic background imported item"),
        },
        PdfImportCandidate {
            path: directory
                .join("missing-background.pdf")
                .to_str()
                .ok_or("missing PDF path")?
                .into(),
            draft: fixture::draft("Synthetic compensated missing PDF"),
        },
    ];
    let initial = service
        .start_pdf_import(store.clone(), key.into(), candidates.clone())
        .await?;
    require(
        initial.source_kind == "pdf"
            && initial.state == "queued"
            && initial.total_count == 2
            && initial.completed_count == 0
            && initial.results.len() == 2
            && initial.results.iter().all(|r| r.state == "pending"),
        "PDF header was not original queued/pending snapshot",
    )?;
    for candidate in &candidates {
        let raw = serde_json::to_string(candidate)?;
        require(
            initial
                .results
                .iter()
                .any(|r| r.input_label == candidate.path && r.raw_input == raw),
            "PDF header changed original candidate audit/id",
        )?;
    }
    let final_job = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let job = get_job(store, service, key, &initial.id).await?;
            if !matches!(job.state.as_str(), "queued" | "running") {
                return Ok::<_, Error>(job);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "PDF finite terminal polling exceeded 90 seconds")??;
    let imported = final_job
        .results
        .iter()
        .find(|r| r.state == "imported")
        .ok_or("missing imported PDF result")?;
    let failed = final_job
        .results
        .iter()
        .find(|r| r.state == "failed")
        .ok_or("missing failed PDF result")?;
    let id = imported
        .item_id
        .as_deref()
        .ok_or("imported PDF result lost item id")?;
    require(
        final_job.state == "completed_with_errors"
            && final_job.completed_count == 2
            && id != fixture::PAPER
            && failed.item_id.is_none()
            && failed.error_message.is_some(),
        "PDF per-item terminal/identity semantics changed",
    )?;
    let db = store.connection()?;
    let compensated: i64 = db.query_row(
        "select count(*) from bibliographic_items where title='Synthetic compensated missing PDF'",
        [],
        |r| r.get(0),
    )?;
    let original: String = db.query_row(
        "select title from bibliographic_items where id=?1",
        [fixture::PAPER],
        |r| r.get(0),
    )?;
    require(
        compensated == 0 && original == "Synthetic attachment target",
        "PDF did not reset draft id or compensate failed attachment",
    )?;
    drop(db);
    let before_cancel = readback::snapshot(store)?;
    let rejection = match service
        .apply_import_jobs(
            store.shared_agent_connection(),
            key.into(),
            ImportJobsRequestV1::Cancel {
                id: initial.id.clone(),
            },
        )
        .await
    {
        Err(error) => error,
        Ok(_) => return Err("terminal PDF job unexpectedly accepted cancellation".into()),
    };
    let cancel_diagnostic = diagnostic(&rejection, key, "not_found", true)?;
    let progress = rejection
        .import_jobs
        .as_ref()
        .ok_or("terminal Cancel lost typed progress")?;
    require(
        progress.phase == "cancel"
            && matches!(
                progress.outcome,
                crate::runtime_plugins::literature_import_jobs::SqlOutcome::Returned
            )
            && progress.job_id.as_deref() == Some(initial.id.as_str()),
        "terminal Cancel lost its own returned-statement progress",
    )?;
    uuid::Uuid::parse_str(
        rejection
            .request_id
            .as_deref()
            .ok_or("terminal Cancel lost actual Native request")?,
    )?;
    let after_cancel = get_job(store, service, key, &initial.id).await?;
    require(
        after_cancel == final_job && readback::snapshot(store)? == before_cancel,
        "terminal Cancel rejection changed the completed job or fixed SQL prefix",
    )?;
    Ok(
        json!({"initial":initial,"terminal":final_job,"afterRejectedTerminalCancel":after_cancel,
        "terminalCancelRejection":cancel_diagnostic,
        "cancelProgressScope":"This Cancel request's progress, not the original header's committed state",
        "cancelScope":"Real shared Cancel rejects a terminal job without changing SQL; no in-flight cancellation claim",
        "polling":{"deadlineSeconds":90,"intervalMilliseconds":20},"originalCandidateAudit":candidates}),
    )
}
