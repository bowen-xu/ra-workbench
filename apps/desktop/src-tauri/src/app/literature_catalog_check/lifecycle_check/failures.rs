use super::*;
use std::sync::{Arc, Mutex};

struct Expected {
    code: &'static str,
    phase: &'static str,
    state: &'static str,
    sqlite: Option<i32>,
}

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: Request,
    expected: Expected,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let error = service
        .apply_literature_lifecycle(store.shared_agent_connection(), key.into(), request.clone())
        .await
        .err()
        .ok_or("rejected lifecycle unexpectedly succeeded")?;
    let wire = serde_json::to_value(&error)?;
    if error.code != expected.code
        || error.plugin_id != "literature_library"
        || error.package_key.as_deref() != Some(key)
        || error.operation_id.is_some()
        || wire["literatureLifecycle"]["phase"] != expected.phase
        || wire["literatureLifecycle"]["outcome"]["state"] != expected.state
        || error
            .data_error
            .as_ref()
            .and_then(|cause| cause.sqlite_extended_code)
            != expected.sqlite
        || error.cleanup_error.is_some()
        || error.exit_code.is_some()
        || error.signal.is_some()
    {
        return Err(
            format!("lifecycle error lost its original outcome/phase/cause: {wire}").into(),
        );
    }
    for identity in [error.instance_id.as_deref(), error.request_id.as_deref()] {
        if identity.is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("lifecycle failure lost actual native instance/request identity".into());
        }
    }
    if wire.to_string().contains(fixture::PRIVATE) || wire.to_string().contains(fixture::BODY) {
        return Err("lifecycle diagnostic exposed synthetic private text".into());
    }
    if !store.connection()?.is_autocommit() {
        return Err("lifecycle left an unsettled SQL transaction".into());
    }
    Ok(json!({"request":request,"error":wire,"before":before,"after":readback::snapshot(store)?}))
}

fn unchanged(value: &Value) -> Result<(), Error> {
    if value["before"] != value["after"] {
        return Err("rejected transactional lifecycle changed fixed SQL state".into());
    }
    Ok(())
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Vec<Value>, Error> {
    let mut results = vec![];
    for (request, phase, state) in [
        (
            Request::TrashMany { item_ids: vec![] },
            "preparation",
            "notDispatched",
        ),
        (
            Request::Archive {
                item_id: "lifecycle-missing".into(),
            },
            "statement-result",
            "statementReturned",
        ),
        (
            Request::TrashMany {
                item_ids: vec!["lifecycle-batch-c".into(), "lifecycle-archived".into()],
            },
            "transaction",
            "rolledBack",
        ),
    ] {
        let failure = rejected(
            store,
            service,
            key,
            request,
            Expected {
                code: "not_found",
                phase,
                state,
                sqlite: None,
            },
        )
        .await?;
        unchanged(&failure)?;
        results.push(failure);
    }
    let attempts = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = attempts.clone();
    {
        let db = store.connection()?;
        db.create_scalar_function(
            "test_lifecycle_attempt",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |ctx| {
                observed
                    .lock()
                    .map_err(|_| {
                        rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(
                            "synthetic lifecycle observer poisoned",
                        )))
                    })?
                    .push(ctx.get::<String>(0)?);
                Ok(0_i64)
            },
        )?;
        db.execute_batch("create temp trigger lifecycle_batch_failure before update of trashed_at on bibliographic_items
            when old.id like 'lifecycle-batch-%' begin select test_lifecycle_attempt(old.id);
            select case when old.id='lifecycle-batch-b' then raise(abort,'private synthetic lifecycle trigger fault') end;end;")?;
    }
    let failed = rejected(
        store,
        service,
        key,
        Request::TrashMany {
            item_ids: vec![
                "lifecycle-batch-c".into(),
                "lifecycle-batch-b".into(),
                "lifecycle-batch-a".into(),
                "lifecycle-batch-a".into(),
            ],
        },
        Expected {
            code: "storage_error",
            phase: "transaction",
            state: "rolledBack",
            sqlite: Some(1811),
        },
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger lifecycle_batch_failure")?;
    if *attempts
        .lock()
        .map_err(|_| "synthetic lifecycle observer poisoned")?
        != ["lifecycle-batch-a", "lifecycle-batch-b"]
    {
        return Err("batch no longer stops at the original ordered first error".into());
    }
    unchanged(&failed)?;
    results.push(json!({"failure":failed,"attemptOrder":attempts.lock().map_err(|_|"synthetic observer poisoned")?.clone()}));
    for (projects, sqlite) in [
        (
            vec!["lifecycle-project-a".into(), "lifecycle-project-a".into()],
            1555,
        ),
        (
            vec![
                "lifecycle-project-a".into(),
                "lifecycle-project-missing".into(),
            ],
            787,
        ),
    ] {
        let failure = rejected(
            store,
            service,
            key,
            Request::SetProjects {
                item_id: fixture::MAIN.into(),
                project_ids: projects,
            },
            Expected {
                code: "storage_error",
                phase: "transaction",
                state: "rolledBack",
                sqlite: Some(sqlite),
            },
        )
        .await?;
        unchanged(&failure)?;
        results.push(failure);
    }
    store.connection()?.execute_batch("create trigger lifecycle_single_fail before update of archived_at on bibliographic_items
        when old.id='lifecycle-fail' begin insert into synthetic_lifecycle_effects(item_id,kind) values(old.id,'fail');
        select raise(fail,'private synthetic lifecycle trigger fault');end;")?;
    let failure = rejected(
        store,
        service,
        key,
        Request::Archive {
            item_id: fixture::FAIL.into(),
        },
        Expected {
            code: "storage_error",
            phase: "statement",
            state: "statementFailed",
            sqlite: Some(1811),
        },
    )
    .await?;
    store
        .connection()?
        .execute_batch("drop trigger lifecycle_single_fail")?;
    let mut before = failure["before"]
        .as_object()
        .ok_or("missing before")?
        .clone();
    let mut after = failure["after"].as_object().ok_or("missing after")?.clone();
    let old = before.remove("effects").ok_or("missing old effects")?;
    let effects = after.remove("effects").ok_or("missing effects")?;
    if before != after
        || old != json!([])
        || effects != json!([{"rowid":1,"item_id":fixture::FAIL,"kind":"fail"}])
    {
        return Err("autocommit FAIL did not retain exactly its original trigger effect".into());
    }
    readback::item(store, fixture::FAIL, false, false)?;
    results.push(json!({"singleFail":failure,"originalAutocommitTriggerEffectRetained":true}));
    Ok(results)
}
