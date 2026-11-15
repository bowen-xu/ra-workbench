use super::*;

struct Expected {
    code: &'static str,
    phase: &'static str,
    sqlite: Option<i32>,
}

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    request: Request,
    expected: Expected,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let error = service
        .apply_import_jobs(store.shared_agent_connection(), key.into(), request.clone())
        .await
        .err()
        .ok_or("synthetic import rejection unexpectedly succeeded")?;
    let value = serde_json::to_value(&error)?;
    if error.code != expected.code
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || value["importJobs"]["phase"] != expected.phase
        || error
            .data_error
            .as_ref()
            .and_then(|e| e.sqlite_extended_code)
            != expected.sqlite
        || error.cleanup_error.is_some()
        || error.exit_code.is_some()
        || error.signal.is_some()
    {
        return Err(
            format!("import diagnostic lost original phase/cause/identity: {value}").into(),
        );
    }
    for id in [error.instance_id.as_deref(), error.request_id.as_deref()] {
        if id.is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("import error lost actual instance/request identity".into());
        }
    }
    match &request {
        Request::Get { id } | Request::Cancel { id } if value["importJobs"]["jobId"] != *id => {
            return Err("import error lost the requested job identity".into());
        }
        Request::Bibtex { drafts }
            if !drafts.is_empty()
                && value["importJobs"]["jobId"]
                    .as_str()
                    .is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) =>
        {
            return Err("failed BibTeX operation lost its allocated job identity".into());
        }
        _ => {}
    }
    if value.to_string().contains(fixture::PRIVATE) {
        return Err("public import diagnostic exposed private synthetic SQL trigger text".into());
    }
    readback::assert_note(store, note)?;
    Ok(json!({"request":request,"error":value,"before":before,"after":readback::snapshot(store)?}))
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    calls: &mut Vec<Value>,
) -> Result<Vec<Value>, Error> {
    let mut failures = vec![];
    for (request, code, phase) in [
        (
            Request::Bibtex { drafts: vec![] },
            "invalid_input",
            "preparation",
        ),
        (
            Request::Get {
                id: "import-cli-missing".into(),
            },
            "not_found",
            "job-read",
        ),
        (
            Request::Cancel {
                id: fixture::DONE.into(),
            },
            "not_found",
            "cancel",
        ),
        (
            Request::Cancel {
                id: "import-cli-missing".into(),
            },
            "not_found",
            "cancel",
        ),
    ] {
        let failure = rejected(
            store,
            service,
            key,
            note,
            request,
            Expected {
                code,
                phase,
                sqlite: None,
            },
        )
        .await?;
        if failure["before"] != failure["after"] {
            return Err("empty/missing/finished import rejection changed fixed data".into());
        }
        failures.push(failure);
    }
    store.connection()?.execute_batch("create temp trigger import_cli_header_abort before insert on import_jobs
        when new.source_kind='bibtex' begin select raise(abort,'private synthetic import-job statement fault');end;")?;
    let header = rejected(
        store,
        service,
        key,
        note,
        Request::Bibtex {
            drafts: vec![fixture::draft(
                "import-cli-header-fault",
                "Header fault draft",
            )],
        },
        Expected {
            code: "storage_error",
            phase: "header",
            sqlite: Some(1811),
        },
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger import_cli_header_abort;")?;
    if header["before"] != header["after"]
        || header["error"]["importJobs"]["headerCommitted"] != false
        || header["error"]["importJobs"]["outcome"] != "rolledBack"
    {
        return Err("failed header transaction did not retain its complete rollback".into());
    }
    failures.push(header);
    store.connection()?.execute_batch(
        "create temp trigger import_cli_result_fail before update of state on import_results
        when new.item_id='import-cli-result-fault' and new.state='imported' begin
        insert into synthetic_import_jobs_cli_events(phase) values('result FAIL effect');
        select raise(fail,'private synthetic import-job statement fault');end;",
    )?;
    let result = rejected(
        store,
        service,
        key,
        note,
        Request::Bibtex {
            drafts: vec![fixture::draft(
                "import-cli-result-fault",
                "Result fault saved draft",
            )],
        },
        Expected {
            code: "storage_error",
            phase: "result-update",
            sqlite: Some(1811),
        },
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger import_cli_result_fail;")?;
    let result_job = result["error"]["importJobs"]["jobId"]
        .as_str()
        .ok_or("result fault lost job id")?;
    if result["error"]["importJobs"]["headerCommitted"] != true
        || result["error"]["importJobs"]["outcome"] != "statementFailed"
        || result["error"]["importJobs"]["completedResultUpdates"] != 0
        || result["error"]["importJobs"]["lastSave"]["status"] != "committed"
        || result["error"]["importJobs"]["lastSave"]["itemId"] != "import-cli-result-fault"
        || result["error"]["importJobs"]["resultId"]
            .as_str()
            .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err("result fault lost original first-save commit/progress".into());
    }
    let actual_result_id: String = store.connection()?.query_row(
        "select id from import_results where job_id=?1",
        [result_job],
        |row| row.get(0),
    )?;
    if result["error"]["importJobs"]["resultId"] != actual_result_id {
        return Err("result fault diagnostic refers to another pending result".into());
    }
    let view = job(apply(
        store,
        service,
        key,
        note,
        Request::Get {
            id: result_job.into(),
        },
        calls,
    )
    .await?)?;
    if view.state != "running"
        || view.completed_count != 0
        || view.results.len() != 1
        || view.results[0].state != "pending"
        || view.results[0].item_id.is_some()
        || readback::value::<i64>(
            store,
            "select count(*) from bibliographic_items where id='import-cli-result-fault'",
        )? != 1
        || readback::value::<i64>(
            store,
            "select count(*) from synthetic_import_jobs_cli_events where phase='result FAIL effect'",
        )? != 1
    {
        return Err(
            "result FAIL rolled back prior header/save or its own original trigger effect".into(),
        );
    }
    failures.push(result);
    store.connection()?.execute_batch("create temp trigger import_cli_finish_abort before update of state on import_jobs
        when old.source_kind='bibtex' and new.state='completed' begin select raise(abort,'private synthetic import-job statement fault');end;")?;
    let finish = rejected(
        store,
        service,
        key,
        note,
        Request::Bibtex {
            drafts: vec![fixture::draft(
                "import-cli-finish-fault",
                "Finish fault saved draft",
            )],
        },
        Expected {
            code: "storage_error",
            phase: "finish-update",
            sqlite: Some(1811),
        },
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger import_cli_finish_abort;")?;
    let finish_job = finish["error"]["importJobs"]["jobId"]
        .as_str()
        .ok_or("finish fault lost job id")?;
    if finish["error"]["importJobs"]["headerCommitted"] != true
        || finish["error"]["importJobs"]["outcome"] != "statementFailed"
        || finish["error"]["importJobs"]["completedResultUpdates"] != 1
        || !finish["error"]["importJobs"]["resultId"].is_null()
        || finish["error"]["importJobs"]["lastSave"]["itemId"] != "import-cli-finish-fault"
    {
        return Err("finish fault lost committed result progress".into());
    }
    let view = job(apply(
        store,
        service,
        key,
        note,
        Request::Get {
            id: finish_job.into(),
        },
        calls,
    )
    .await?)?;
    if view.state != "running"
        || view.completed_count != 0
        || view.results.len() != 1
        || view.results[0].state != "imported"
        || view.results[0].item_id.as_deref() != Some("import-cli-finish-fault")
        || readback::value::<i64>(
            store,
            "select count(*) from bibliographic_items where id='import-cli-finish-fault'",
        )? != 1
    {
        return Err(
            "finish failure rolled back the original saved item or returned result update".into(),
        );
    }
    failures.push(finish);
    let requests = failures
        .iter()
        .map(|failure| failure["error"]["requestId"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    if requests.len() != failures.len() {
        return Err("separate import operations reused a native request identity".into());
    }
    Ok(failures)
}
