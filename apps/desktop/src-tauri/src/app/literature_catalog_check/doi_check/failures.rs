use super::*;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    calls: &mut Vec<Value>,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let fixture::Http {
        url, release, task, ..
    } = fixture::http("404 Not Found", "synthetic missing DOI".into()).await?;
    let _ = release.send(());
    let response = fixture::preview(
        service,
        store.shared_agent_connection(),
        key.into(),
        fixture::RAW.into(),
        None,
        url,
    )
    .await;
    fixture::check_request(&task.await??)?;
    let error = match response {
        Err(error) => error,
        Ok(_) => return Err("synthetic missing resolver unexpectedly produced a preview".into()),
    };
    let progress = error
        .import_jobs
        .as_ref()
        .ok_or("DOI failure lost its original committed prefix")?;
    let job_id = progress
        .job_id
        .as_deref()
        .ok_or("DOI failure lost job ID")?;
    let result_id = progress
        .result_id
        .as_deref()
        .ok_or("DOI failure lost result ID")?;
    if error.code != "connector_error"
        || error.message != "DOI 元数据请求失败：没有找到 DOI 元数据，请检查 DOI 是否正确"
        || error.package_key.as_deref() != Some(key)
        || error.instance_id.as_deref().is_none_or(str::is_empty)
        || error.request_id.as_deref().is_none_or(str::is_empty)
        || progress.header_committed != Some(true)
    {
        return Err(
            "resolver failure lost its original error or actual Native/job identity".into(),
        );
    }
    let job = get_job(store, service, key, job_id, calls).await?;
    if job.state != "failed"
        || job.completed_count != 1
        || job.results.len() != 1
        || job.results[0].state != "failed"
        || job.results[0].error_message.as_deref() != Some(error.message.as_str())
    {
        return Err(
            "resolver failure did not retain the committed header and failed result".into(),
        );
    }
    let after = readback::snapshot(store)?;
    for name in [
        "items",
        "creators",
        "creatorLinks",
        "identifiers",
        "tags",
        "tagLinks",
        "provenance",
        "candidates",
        "fts",
        "duplicates",
        "notePages",
        "noteRevisions",
        "noteLinks",
    ] {
        if before[name] != after[name] {
            return Err("resolver failure changed unrelated data or the Core Note".into());
        }
    }
    if !after["results"]
        .as_array()
        .ok_or("missing failed results")?
        .iter()
        .any(|r| r["id"] == result_id)
    {
        return Err("known failed DOI result does not exist".into());
    }
    Ok(json!({"error":error,"job":job,"sqlSnapshot":after}))
}
