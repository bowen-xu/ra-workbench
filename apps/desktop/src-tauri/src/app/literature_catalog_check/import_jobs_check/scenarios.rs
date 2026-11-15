use super::*;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    calls: &mut Vec<Value>,
) -> Result<Value, Error> {
    let mut first = fixture::draft("import-cli-main", "  Original raw BibTeX title  ");
    first.bibtex_key = "  RawKey  ".into();
    first.source_kind = Some("caller-original-source".into());
    first.doi = "https://doi.org/10.1000/BIBTEX".into();
    first.tags = vec![" first ".into(), " Second ".into()];
    let mut label = fixture::draft("import-cli-label", "  Raw title label  ");
    label.bibtex_key = " \t ".into();
    let empty = fixture::draft("import-cli-empty", " \t ");
    let mut unsupported = fixture::draft("import-cli-unsupported", "Unsupported raw type");
    unsupported.item_type = "synthetic-unsupported".into();
    let failed = fixture::draft("import-cli-save-fault", "Synthetic save failure entry");
    let drafts = vec![first, label, empty, unsupported, failed];
    store.connection()?.execute_batch("create temp trigger import_cli_item_abort before insert on bibliographic_items
        when new.id='import-cli-save-fault' begin select raise(abort,'synthetic original item-save failure');end;")?;
    let imported = job(apply(
        store,
        service,
        key,
        note,
        Request::Bibtex {
            drafts: drafts.clone(),
        },
        calls,
    )
    .await?)?;
    store
        .connection()?
        .execute_batch("drop trigger import_cli_item_abort;")?;
    if imported.source_kind != "bibtex"
        || imported.state != "completed_with_errors"
        || imported.total_count != 5
        || imported.completed_count != 5
        || imported.results.len() != 5
        || imported.cancellation_requested
    {
        return Err("mixed BibTeX job changed original aggregate/count/source semantics".into());
    }
    for (index, (input, result)) in drafts.iter().zip(&imported.results).enumerate() {
        let expected_label = if index == 0 {
            "  RawKey  "
        } else {
            input.title.as_str()
        };
        if result.input_label != expected_label || result.raw_input != serde_json::to_string(input)?
        {
            return Err("BibTeX raw label, caller source or draft JSON changed before save".into());
        }
        if index < 2 {
            if result.state != "imported"
                || result.item_id != input.id
                || result.error_message.is_some()
            {
                return Err("valid BibTeX entry did not commit its original item".into());
            }
        } else {
            let expected = match index {
                2 => "标题不能为空".into(),
                3 => "不支持的文献类型：synthetic-unsupported".into(),
                _ => format!("文献数据库操作失败：{}", fixture::BUSINESS_FAILURE),
            };
            if result.state != "failed"
                || result.item_id.is_some()
                || result.error_message.as_deref() != Some(expected.as_str())
            {
                return Err(
                    "failed BibTeX entry lost its original business error or nullable item".into(),
                );
            }
        }
    }
    let stored:(String,String,String,String)=store.connection()?.query_row(
        "select b.title,b.bibtex_key,i.normalized_value,i.display_value from bibliographic_items b join identifiers i on i.item_id=b.id and i.kind='doi' where b.id='import-cli-main'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if stored
        != (
            "Original raw BibTeX title".into(),
            "RawKey".into(),
            "10.1000/bibtex".into(),
            "10.1000/BIBTEX".into(),
        )
    {
        return Err("BibTeX canonical item fields or shared DOI changed".into());
    }
    if readback::provenance(store, "import-cli-main", "title")? != ("bibtex".into(), None, false) {
        return Err("BibTeX caller source incorrectly replaced forced original provenance".into());
    }
    if readback::value::<i64>(
        store,
        "select count(*) from bibliographic_items where id in('import-cli-empty','import-cli-unsupported','import-cli-save-fault')",
    )? != 0
    {
        return Err("failed BibTeX entries left item rows".into());
    }
    let reread = job(apply(
        store,
        service,
        key,
        note,
        Request::Get {
            id: imported.id.clone(),
        },
        calls,
    )
    .await?)?;
    if reread != imported {
        return Err("Get did not preserve complete returned BibTeX job/results".into());
    }
    for (id, state, source, count) in [
        (fixture::DOI, "queued", "doi", 1),
        (fixture::PDF, "running", "pdf", 2),
        (fixture::DONE, "completed", "bibtex", 1),
    ] {
        let view = job(apply(
            store,
            service,
            key,
            note,
            Request::Get { id: id.into() },
            calls,
        )
        .await?)?;
        if view.id != id
            || view.state != state
            || view.source_kind != source
            || view.results.len() != count
        {
            return Err("shared DOI/PDF job getter changed original fields".into());
        }
        if id == fixture::DOI
            && (!view.results[0].raw_input.is_empty()
                || view.results[0].input_label != "  Original DOI input  "
                || view.results[0].item_id.is_some()
                || view.results[0].error_message.is_some())
        {
            return Err("job getter lost coalesced raw input or explicit null fields".into());
        }
    }
    let Response::Jobs { jobs } = apply(store, service, key, note, Request::List {}, calls).await?
    else {
        return Err("List returned a different import result kind".into());
    };
    if jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>()
        != [
            imported.id.as_str(),
            fixture::DOI,
            fixture::PDF,
            fixture::DONE,
        ]
    {
        return Err("shared import job list changed original descending order".into());
    }
    for (id, state) in [(fixture::DOI, "queued"), (fixture::PDF, "running")] {
        if !matches!(
            apply(
                store,
                service,
                key,
                note,
                Request::Cancel { id: id.into() },
                calls
            )
            .await?,
            Response::Done {}
        ) {
            return Err("Cancel returned a different import result kind".into());
        }
        let view = job(apply(
            store,
            service,
            key,
            note,
            Request::Get { id: id.into() },
            calls,
        )
        .await?)?;
        if !view.cancellation_requested || view.state != state {
            return Err("Cancel replaced the original queued/running state instead of setting the request flag".into());
        }
    }
    Ok(
        json!({"job":imported,"rawDrafts":drafts,"validEntries":2,"failedEntries":3,
        "originalItemSaveErrorIsBusinessContent":true,"sharedDoiPdfViews":true,"cancellationFlagOnly":true}),
    )
}
