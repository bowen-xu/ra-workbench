use super::*;
use rusqlite::functions::FunctionFlags;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct Startup {
    pub note: String,
    pub document: Value,
    pub queue: Arc<AtomicUsize>,
    pub state: NalState,
    pub metadata: Value,
    pub failures: Vec<Value>,
}

struct Expected {
    code: &'static str,
    message: &'static str,
    phase: &'static str,
}

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    mutation: NalMutation,
    revision: i64,
    expected: Expected,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let error = service
        .mutate_nal_network(
            store.shared_agent_connection(),
            key.into(),
            mutation,
            revision,
        )
        .await
        .err()
        .ok_or("synthetic mutation failure unexpectedly succeeded")?;
    let value = serde_json::to_value(&error)?;
    if error.code != expected.code
        || error.message != expected.message
        || value["mutationOutcome"]["status"] != "rolledBack"
        || value["mutationProblem"]["phase"] != expected.phase
        || error.package_key.as_deref() != Some(key)
        || error.instance_id.is_none()
        || error.request_id.is_none()
        || value.get("mutationRollbackError").is_some()
        || readback::snapshot(store)? != before
    {
        return Err(format!(
            "mutation failure lost exact settlement, safe cause, identity or rollback: {error:?}"
        )
        .into());
    }
    // Unknown is deliberately not accepted as a rolled-back CAS/domain error.
    Ok(value)
}

pub(super) async fn startup(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Startup, Error> {
    let (note, queue) = fixture::seed(store)?;
    let document = fixture::note(store, &note)?;
    let empty = || NalMutation::AddStatements { statements: vec![] };
    let malformed = rejected(
        store,
        service,
        key,
        empty(),
        i64::MIN,
        Expected {
            code: "invalid_input",
            message: "关系数据结构无效",
            phase: "startup",
        },
    )
    .await?;
    let malformed_attempts = queue.load(Ordering::SeqCst);
    if malformed_attempts != 8 || fixture::note(store, &note)? != document {
        return Err(format!("original malformed startup did not roll back eight queue attempts and preserve the Core note: {malformed_attempts}").into());
    }
    // Repair only the original fixture's malformed Fact; its annotation cascades.
    if store
        .connection()?
        .execute("delete from nal_statements where id=?1", [fixture::BAD])?
        != 1
    {
        return Err("synthetic malformed Fact repair did not affect one row".into());
    }
    let conflict = rejected(
        store,
        service,
        key,
        empty(),
        i64::MIN,
        Expected {
            code: "version_conflict",
            message: "关系已变化，请刷新后重试",
            phase: "revisionCheck",
        },
    )
    .await?;
    let conflict_attempts = queue.load(Ordering::SeqCst) - malformed_attempts;
    if conflict_attempts != 8 || fixture::note(store, &note)? != document {
        return Err(
            "CAS did not follow complete startup and precede empty-batch validation".into(),
        );
    }
    let before = queue.load(Ordering::SeqCst);
    let state = read(store, service, key).await?;
    let committed_attempts = queue.load(Ordering::SeqCst) - before;
    let metadata = readback::metadata(store)?;
    if committed_attempts != 8
        || metadata["queuedJobs"] != 8
        || state.concepts.len() != 10
        || state.statements.len() != 8
        || readback::scalar(
            store,
            "select legacy_imported=1 and relation_semantics_version=1 from nal_state where id=1",
        )? != 1
        || fixture::note(store, &note)? != document
    {
        return Err(
            "the original complete bootstrap branches/queue/metadata/Core note changed".into(),
        );
    }
    Ok(Startup {
        note,
        document,
        queue,
        state,
        metadata,
        failures: vec![
            json!({"failure":malformed,"queueInsertAttempts":malformed_attempts,"sameCompleteSqlSnapshot":true}),
            json!({"failure":conflict,"queueInsertAttempts":conflict_attempts,"sameCompleteSqlSnapshot":true}),
        ],
    })
}

pub(super) async fn ordered_and_final(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    state: &NalState,
    target: &str,
) -> Result<Vec<Value>, Error> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    {
        let db = store.connection()?;
        db.create_scalar_function(
            "observe_mutation_cli_statement",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            },
        )?;
        db.execute_batch(
            "create temp trigger observe_mutation_cli_statement after insert on nal_statements
            when new.id in ('mutation-prefix-first','mutation-final-load','mutation-revision-fault')
            begin select observe_mutation_cli_statement(); end;
            create temp trigger fail_mutation_cli_first before insert on nal_statements
            when new.id='mutation-prefix-first' begin select observe_mutation_cli_statement();
            select raise(abort,'synthetic first mutation insert failure'); end;",
        )?;
    }
    let first = statement("mutation-prefix-first", term(target), paper());
    let mut later = statement("mutation-prefix-later", term(target), term(target));
    later.origin = "legacy".into();
    let first_sql = rejected(
        store,
        service,
        key,
        NalMutation::AddStatements {
            statements: vec![first.clone(), later],
        },
        state.revision,
        Expected {
            code: "storage_error",
            message: "关系存储失败，请重试",
            phase: "statementInsert",
        },
    )
    .await;
    store
        .connection()?
        .execute_batch("drop trigger fail_mutation_cli_first")?;
    let first_sql = first_sql?;
    let first_attempts = attempts.load(Ordering::SeqCst);
    if first_attempts != 1 {
        return Err("later identity rejection skipped the first actual INSERT error".into());
    }
    let missing = statement(
        "mutation-prefix-later",
        term("mutation-missing-concept"),
        term(target),
    );
    let later_term = rejected(
        store,
        service,
        key,
        NalMutation::AddStatements {
            statements: vec![first, missing],
        },
        state.revision,
        Expected {
            code: "invalid_input",
            message: "概念或活跃文献已不存在，请刷新",
            phase: "termRead",
        },
    )
    .await?;
    let later_attempts = attempts.load(Ordering::SeqCst) - first_attempts;
    if later_attempts != 1 {
        return Err("later term rejection did not follow one attempted INSERT".into());
    }
    let mut results = vec![
        json!({"failure":first_sql,"firstInsertAttempts":first_attempts,"earlierSqlWon":true}),
        json!({"failure":later_term,"firstInsertAttempts":later_attempts,"wholeBatchRolledBack":true}),
    ];
    for (id, trigger, sql, expected) in [
        ("mutation-final-load", "corrupt_mutation_cli_final",
         "create temp trigger corrupt_mutation_cli_final after insert on nal_statements
          when new.id='mutation-final-load' begin
          insert into nal_statements(id,subject,predicate,origin)
          values('mutation-invalid-final-fact','{\"kind\":\"variable\",\"name\":\"not-a-fact\"}',new.predicate,'human'); end;",
         Expected { code:"invalid_input", message:"关系数据结构无效", phase:"finalLoad" }),
        ("mutation-revision-fault", "fail_mutation_cli_revision",
         "create temp trigger fail_mutation_cli_revision before update of revision on nal_state
          when exists(select 1 from nal_statements where id='mutation-revision-fault')
          begin select raise(abort,'synthetic mutation final revision failure'); end;",
         Expected { code:"storage_error", message:"关系存储失败，请重试", phase:"revision" }),
    ] {
        let before = attempts.load(Ordering::SeqCst);
        store.connection()?.execute_batch(sql)?;
        let result = rejected(store, service, key,
            NalMutation::AddStatements { statements:vec![statement(id, term(target), paper())] },
            state.revision, expected).await;
        // Trigger names come only from the two fixed declarations above.
        store.connection()?.execute_batch(&format!("drop trigger {trigger}"))?;
        let failure = result?;
        let inserted = attempts.load(Ordering::SeqCst) - before;
        if inserted != 1 { return Err("terminal mutation fault did not follow its actual INSERT".into()); }
        results.push(json!({"failure":failure,"statementInsertAttempts":inserted,"wholeMutationRolledBack":true}));
    }
    store
        .connection()?
        .execute_batch("drop trigger observe_mutation_cli_statement")?;
    Ok(results)
}
