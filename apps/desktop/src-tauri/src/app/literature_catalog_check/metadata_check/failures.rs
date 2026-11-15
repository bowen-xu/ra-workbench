use super::*;

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    request: Request,
    expected: (&str, &str),
    expected_sqlite: Option<i32>,
) -> Result<Value, Error> {
    let error = service
        .apply_metadata(store.shared_agent_connection(), key.into(), request.clone())
        .await
        .err()
        .ok_or("synthetic metadata request unexpectedly succeeded")?;
    let wire = serde_json::to_value(&error)?;
    if error.code != expected.0
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || wire["literatureMetadata"]["outcome"]["state"] != expected.1
        || error.cleanup_error.is_some()
        || error.exit_code.is_some()
        || error.signal.is_some()
        || error.operation_id.is_some()
    {
        return Err(format!("metadata diagnostic lost its actual outcome: {wire}").into());
    }
    for identity in [error.instance_id.as_deref(), error.request_id.as_deref()] {
        if identity.is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("metadata dispatched failure lost its real host identity".into());
        }
    }
    if expected.0 == "storage_error"
        && (error
            .data_error
            .as_ref()
            .and_then(|e| e.sqlite_extended_code)
            != expected_sqlite
            || wire.to_string().contains("private metadata trigger body"))
    {
        return Err(
            "metadata SQL failure lost its exact safe SQLite code or exposed its trigger body"
                .into(),
        );
    }
    readback::assert_note(store, note)?;
    Ok(json!({"request":request,"error":wire,"sqlSnapshot":readback::snapshot(store)?}))
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Vec<Value>, Error> {
    let mut failures = Vec::new();
    let before = readback::snapshot(store)?;
    let empty = rejected(
        store,
        service,
        key,
        note,
        Request::Update {
            item_id: fixture::MAIN.into(),
            patch: Patch::Title(" \t ".into()),
        },
        ("invalid_input", "rolledBack"),
        None,
    )
    .await?;
    if empty["error"]["message"] != "标题不能为空" || readback::snapshot(store)? != before {
        return Err("empty title rejection changed its original message or rollback".into());
    }
    failures.push(empty);
    let empty_confirm = rejected(
        store,
        service,
        key,
        note,
        Request::Confirm {
            item_id: fixture::MAIN.into(),
            field_names: vec![],
        },
        ("invalid_input", "notStarted"),
        None,
    )
    .await?;
    if empty_confirm["error"]["message"] != "元数据确认字段无效：至少选择一个字段"
        || readback::snapshot(store)? != before
    {
        return Err("empty confirmation changed its original validation boundary".into());
    }
    failures.push(empty_confirm);
    let repeated = rejected(
        store,
        service,
        key,
        note,
        Request::Reject {
            candidate_id: "metadata-cli-reject-null".into(),
        },
        ("not_found", "statementReturned"),
        None,
    )
    .await?;
    if repeated["error"]["message"] != "找不到文献或附件"
        || repeated["error"]["literatureMetadata"]["outcome"]["affectedRows"] != 0
        || readback::snapshot(store)? != before
    {
        return Err("repeat Reject changed its actual zero-row result".into());
    }
    failures.push(repeated);
    failures.push(full_second(store, service, key, note).await?);
    failures.push(reject_fail(store, service, key, note).await?);
    failures.push(hidden_projection(store, service, key, note).await?);
    failures.push(duplicate_tags(store, service, key, note).await?);
    Ok(failures)
}

async fn hidden_projection(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    store.connection()?.execute_batch("insert into bibliographic_items(id,item_type,title,archived_at,created_at,updated_at) values('metadata-cli-hidden','journalArticle','private hidden metadata title','2026','2026','2026');")?;
    fixture::candidate(
        store,
        "metadata-cli-hidden-candidate",
        Some("metadata-cli-hidden"),
        &serde_json::to_string(&fixture::draft("private hidden metadata candidate"))?,
        false,
    )?;
    let before = readback::snapshot(store)?;
    let failure = rejected(
        store,
        service,
        key,
        note,
        Request::FieldsApply {
            candidate_id: "metadata-cli-hidden-candidate".into(),
            decisions: vec![],
            candidate_trust: Trust::Reviewable,
        },
        ("not_found", "notStarted"),
        None,
    )
    .await?;
    let wire = &failure["error"];
    if wire["message"] != "找不到文献或附件"
        || wire["literatureMetadata"]["phase"] != "projection"
        || wire["literatureMetadata"]["itemId"] != "metadata-cli-hidden"
        || wire.to_string().contains("private hidden metadata")
        || readback::snapshot(store)? != before
    {
        return Err(
            "hidden Fields projection lost known identity or changed SQL/private diagnostics"
                .into(),
        );
    }
    Ok(failure)
}

async fn full_second(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    let candidate = fixture::draft("Candidate for late failure");
    fixture::candidate(
        store,
        "metadata-cli-second-fail",
        Some(fixture::PARTIAL),
        &serde_json::to_string(&candidate)?,
        true,
    )?;
    store.connection()?.execute_batch("create temp trigger synthetic_metadata_cli_second before update of state on metadata_candidates when old.id='metadata-cli-second-fail' and new.state='applied' begin insert into synthetic_metadata_cli_events(phase) values('rolled-back-second');select raise(abort,'private metadata trigger body');end;")?;
    let mut draft = fixture::draft(" First phase survives ");
    draft.id = Some("metadata-cli-wrong-caller".into());
    draft.source_kind = Some("user".into());
    let rejected = rejected(
        store,
        service,
        key,
        note,
        Request::FullApply {
            candidate_id: "metadata-cli-second-fail".into(),
            draft: Box::new(draft),
        },
        ("storage_error", "rolledBack"),
        Some(1811),
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger synthetic_metadata_cli_second;")?;
    let error = &rejected["error"]["literatureMetadata"];
    if error["phase"] != "candidate-applied"
        || error["firstSave"] != json!({"status":"committed","itemId":fixture::PARTIAL})
        || error["itemId"] != fixture::PARTIAL
        || readback::value::<String>(
            store,
            "select title from bibliographic_items where id='metadata-cli-partial'",
        )? != "First phase survives"
        || readback::provenance(store, fixture::PARTIAL, "title")?
            != ("crossref".into(), None, false)
        || readback::candidate_state(store, "metadata-cli-second-fail")? != "pending"
        || readback::value::<String>(
            store,
            "select state from import_results where id='metadata-cli-second-fail-result'",
        )? != "pending"
        || readback::value::<String>(
            store,
            "select state from import_jobs where id='metadata-cli-second-fail-job'",
        )? != "interrupted"
        || readback::value::<i64>(
            store,
            "select count(*) from synthetic_metadata_cli_events where phase='rolled-back-second'",
        )? != 0
    {
        return Err("late FullApply rollback lost its distinct committed first save".into());
    }
    Ok(rejected)
}

async fn reject_fail(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    // Valid JSON with the wrong Draft shape: Reject must never decode it.
    fixture::candidate(
        store,
        "metadata-cli-reject-fail",
        Some(fixture::MAIN),
        "[]",
        true,
    )?;
    store.connection()?.execute_batch("create temp trigger synthetic_metadata_cli_reject before update on import_results when old.id='metadata-cli-reject-fail-result' begin insert into synthetic_metadata_cli_events(phase) values('reject-fail-effect');select raise(fail,'private metadata trigger body');end;")?;
    let rejected = rejected(
        store,
        service,
        key,
        note,
        Request::Reject {
            candidate_id: "metadata-cli-reject-fail".into(),
        },
        ("storage_error", "statementFailed"),
        Some(1811),
    )
    .await?;
    let failure = &rejected["error"]["literatureMetadata"];
    if failure["phase"] != "result-cancelled"
        || failure["completedStatements"] != json!(["candidate-rejected"])
        || readback::candidate_state(store, "metadata-cli-reject-fail")? != "rejected"
        || readback::value::<String>(
            store,
            "select state from import_results where id='metadata-cli-reject-fail-result'",
        )? != "pending"
        || readback::value::<String>(
            store,
            "select state from import_jobs where id='metadata-cli-reject-fail-job'",
        )? != "interrupted"
        || readback::value::<i64>(
            store,
            "select count(*) from synthetic_metadata_cli_events where phase='reject-fail-effect'",
        )? != 1
    {
        return Err(
            "Reject FAIL erased prior autocommits, lost the FAIL effect or replayed".into(),
        );
    }
    store
        .connection()?
        .execute_batch("drop trigger synthetic_metadata_cli_reject;")?;
    Ok(rejected)
}

async fn duplicate_tags(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let failure = rejected(
        store,
        service,
        key,
        note,
        Request::Update {
            item_id: fixture::MAIN.into(),
            patch: Patch::Tags(vec![" alpha ".into(), "ALPHA".into()]),
        },
        ("storage_error", "rolledBack"),
        Some(1555),
    )
    .await?;
    if failure["error"]["literatureMetadata"]["phase"] != "item-tag-insert"
        || failure["error"]["literatureMetadata"]["itemId"] != fixture::MAIN
        || failure["error"]["dataError"]["resourceId"] != "literature.catalog-save.item-tag-insert"
        || readback::snapshot(store)? != before
    {
        return Err(
            "duplicate/NOCASE tags did not preserve original PK failure and full rollback".into(),
        );
    }
    Ok(failure)
}
