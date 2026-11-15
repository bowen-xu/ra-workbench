use super::*;
use rusqlite::{
    functions::FunctionFlags,
    hooks::{AuthContext, Authorization},
};

pub(super) async fn call(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: NalAnnotationRequest,
    code: &str,
    message: &str,
    status: &str,
) -> Result<Value, Error> {
    let failure = service
        .ensure_nal_annotation(store.shared_agent_connection(), key.into(), request)
        .await
        .err()
        .ok_or("synthetic annotation failure unexpectedly succeeded")?;
    let value = serde_json::to_value(&failure)?;
    if failure.code != code
        || failure.message != message
        || value["annotationOutcome"]["status"] != status
        || failure.package_key.as_deref() != Some(key)
        || (status == "rolledBack"
            && (failure.instance_id.is_none() || failure.request_id.is_none()))
    {
        return Err(format!(
            "annotation failure lost its original error/settlement/identity: {failure:?}"
        )
        .into());
    }
    Ok(value)
}

pub(super) async fn invalid_before_sql(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let visits = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&visits);
    {
        let db = store.connection()?;
        // Observe without denying Core owner's own materialization SQL when
        // this observer is later removed. The service call itself must perform
        // no SQL before rejecting the original invalid request.
        db.authorizer(Some(move |_: AuthContext<'_>| {
            observed.fetch_add(1, Ordering::SeqCst);
            Authorization::Allow
        }));
    }
    let result = call(
        store,
        service,
        key,
        request(" \t", "Synthetic invalid target"),
        "invalid_input",
        "笔记目标或标题无效",
        "notStarted",
    )
    .await;
    // Capture the service-only count before acquiring the teardown guard:
    // AppStore::connection may legitimately materialize its original owner.
    let observed_sql_actions = visits.load(Ordering::SeqCst);
    store
        .connection()?
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let mut failure = result?;
    if observed_sql_actions != 0 {
        return Err("invalid annotation request reached Core SQL before validation".into());
    }
    failure["observedSqlActions"] = json!(0);
    Ok(failure)
}

pub(super) fn observe_notes(store: &AppStore) -> Result<Arc<AtomicUsize>, Error> {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    let db = store.connection()?;
    db.create_scalar_function(
        "observe_annotation_cli_note",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        },
    )?;
    db.execute_batch(
        "create temp trigger observe_annotation_cli_note_insert after insert on note_pages
         begin select observe_annotation_cli_note(); end;",
    )?;
    Ok(count)
}

pub(super) async fn write_failure(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    link: bool,
    queue: &Arc<AtomicUsize>,
    notes: &Arc<AtomicUsize>,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let queue_before = queue.load(Ordering::SeqCst);
    let notes_before = notes.load(Ordering::SeqCst);
    let (trigger, sql) = if link {
        (
            "fail_annotation_cli_link",
            "create temp trigger fail_annotation_cli_link before insert on nal_annotations
             when new.concept_id='bootstrap-endpoint'
             begin select raise(abort,'synthetic annotation CLI link failure'); end;",
        )
    } else {
        (
            "fail_annotation_cli_revision",
            "create temp trigger fail_annotation_cli_revision before update of revision on nal_state
             when exists(select 1 from nal_annotations where concept_id='bootstrap-endpoint')
             begin select raise(abort,'synthetic annotation CLI final revision failure'); end;",
        )
    };
    store.connection()?.execute_batch(sql)?;
    let result = call(
        store,
        service,
        key,
        request(TARGET, "Synthetic rolled-back Core annotation note"),
        "storage_error",
        "关系存储失败，请重试",
        "rolledBack",
    )
    .await;
    // This identifier is selected only from the two fixed declarations above.
    store
        .connection()?
        .execute_batch(&format!("drop trigger {trigger}"))?;
    let failure = result?;
    let queue_attempts = queue.load(Ordering::SeqCst) - queue_before;
    let note_attempts = notes.load(Ordering::SeqCst) - notes_before;
    if queue_attempts == 0 || note_attempts != 1 || readback::snapshot(store)? != before {
        return Err(format!("annotation SQL failure did not roll back startup/queue and exactly one attempted Core page: queue={queue_attempts}, notes={note_attempts}").into());
    }
    Ok(
        json!({"failure":failure,"startupQueueInsertAttempts":queue_attempts,
        "coreNoteInsertAttempts":note_attempts,"sameCompleteSqlSnapshot":true}),
    )
}
