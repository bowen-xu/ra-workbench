//! Local suspension mechanics. Real SQL bytes and Native execution have separate tests.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_index::{
        decisions::{Cache, Decided, IndexRun},
        failure::{Error, deferred},
    },
};
use research_workbench_native_sdk::semantic_index::rules::{IndexDecisionV1, IndexInputV1};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization, TransactionOperation},
};

fn resources() -> IndexResources {
    IndexResources(BTreeMap::from([(
        IndexSql::RepositoryLoadJob1,
        "select value from fixture order by rowid".into(),
    )]))
}
fn observe(run: &IndexRun<'_>, db: &Connection) {
    db.prepare(run.sql(IndexSql::RepositoryLoadJob1))
        .unwrap()
        .query_map([], |row| run.row(row, |row| row.get::<_, String>(0)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
}
fn input() -> IndexInputV1 {
    IndexInputV1::Status { spaces: Vec::new() }
}
fn decide(run: &IndexRun<'_>) -> Cache {
    let Error::Need(need) = run.decide(input()).unwrap_err() else {
        panic!("missing finite decision");
    };
    Cache(vec![Decided {
        need: *need,
        decision: IndexDecisionV1::Flag { value: false },
        origin: DecisionOrigin {
            package_key: "synthetic-package".into(),
            instance_id: "synthetic-instance".into(),
            native_request_id: "synthetic-request".into(),
        },
    }])
}

#[test]
fn full_original_rows_are_rechecked_even_when_finite_rule_input_is_unchanged() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table fixture(value text); insert into fixture values('before')")
        .unwrap();
    let resources = resources();
    let initial = Cache::default();
    let run = IndexRun::new(&resources, &initial);
    observe(&run, &db);
    let cache = decide(&run);
    let same = IndexRun::new(&resources, &cache);
    observe(&same, &db);
    assert!(matches!(
        same.decide(input()),
        Ok(IndexDecisionV1::Flag { value: false })
    ));
    db.execute("update fixture set value='after'", []).unwrap();
    let changed = IndexRun::new(&resources, &cache);
    observe(&changed, &db);
    assert!(
        matches!(changed.decide(input()), Err(Error::Conflict(origin)) if origin.native_request_id == "synthetic-request")
    );
}

#[test]
fn a_missing_row_is_part_of_the_same_original_query_prefix() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table fixture(value text)")
        .unwrap();
    let resources = resources();
    let initial = Cache::default();
    let run = IndexRun::new(&resources, &initial);
    observe(&run, &db);
    let cache = decide(&run);
    db.execute("insert into fixture values('appeared')", [])
        .unwrap();
    let changed = IndexRun::new(&resources, &cache);
    observe(&changed, &db);
    assert!(matches!(changed.decide(input()), Err(Error::Conflict(_))));
}

#[test]
fn original_row_decoder_failure_precedes_raw_cell_capture() {
    let db = Connection::open_in_memory().unwrap();
    let resources = resources();
    let cache = Cache::default();
    let run = IndexRun::new(&resources, &cache);
    let failure = db
        .query_row("select 1, 'not an integer'", [], |row| {
            run.row(row, |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
        })
        .unwrap_err();
    assert!(matches!(
        failure,
        rusqlite::Error::InvalidColumnType(0, _, _)
    ));
}

#[test]
fn native_need_is_exposed_only_after_original_deferred_rollback_is_confirmed() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table fixture(value text)")
        .unwrap();
    let resources = resources();
    let cache = Cache::default();
    let run = IndexRun::new(&resources, &cache);
    let result = deferred(&mut db, |tx| {
        tx.execute("insert into fixture values('uncommitted')", [])?;
        run.decide(input())
    });
    assert!(matches!(result, Err(Error::Need(_))));
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select count(*) from fixture", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn denied_rollback_returns_recovery_failure_instead_of_a_native_need() {
    let mut db = Connection::open_in_memory().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let resources = resources();
    let cache = Cache::default();
    let run = IndexRun::new(&resources, &cache);
    let failure = deferred(&mut db, |_| run.decide(input())).unwrap_err();
    assert!(failure.needs_recovery());
    let Error::Boundary(failure) = failure else {
        panic!("unconfirmed settlement");
    };
    assert_eq!(failure.outcome, HbsOutcome::Unknown);
    assert_eq!(failure.rollback_error.unwrap().phase, "rollback");
    assert!(!db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    db.execute_batch("rollback").unwrap();
}

#[test]
fn original_invalid_input_code_keeps_actual_native_rejection_identity_after_rollback() {
    let mut db = Connection::open_in_memory().unwrap();
    let resources = resources();
    let empty = Cache::default();
    let first = IndexRun::new(&resources, &empty);
    let mut cache = decide(&first);
    cache.0[0].decision = IndexDecisionV1::Rejected {
        message: "synthetic rejection".into(),
    };
    let run = IndexRun::new(&resources, &cache);
    let failure = deferred(&mut db, |_| run.decide(input())).unwrap_err();
    assert!(!failure.needs_recovery());
    let wire = failure.into_wire("retry", Some("representation"));
    assert_eq!(wire["code"], "invalid_input");
    assert_eq!(wire["representationId"], "representation");
    assert_eq!(wire["nativeFailure"]["outcome"], "rolledBack");
    assert_eq!(
        wire["nativeFailure"]["nativeRequestId"],
        "synthetic-request"
    );
    assert!(db.is_autocommit());
}

#[test]
fn native_technical_failure_uses_the_shared_validation_phase_and_actual_identity() {
    use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
    let mut error = NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "索引结果与原输入不匹配。",
    );
    error.package_key = Some("a".repeat(64));
    error.instance_id = Some("index-instance".into());
    error.request_id = Some("index-request".into());
    let wire = Error::Native {
        error,
        outcome: HbsOutcome::RolledBack,
    }
    .into_wire("retry", Some("representation"));
    let diagnostic = &wire["nativeFailure"];
    assert_eq!(diagnostic["phase"], "validation");
    assert_eq!(diagnostic["outcome"], "rolledBack");
    assert_eq!(diagnostic["packageKey"], "a".repeat(64));
    assert_eq!(diagnostic["instanceId"], "index-instance");
    assert_eq!(diagnostic["nativeRequestId"], "index-request");
    assert_eq!(wire["representationId"], "representation");
    assert!(wire.get("jobId").is_none());
}

#[test]
fn native_retained_job_cannot_replace_any_field_of_the_original_scoped_summary() {
    let resources = IndexResources(BTreeMap::from([
        (IndexSql::RepositoryQueueRetry1, "select 'record', 'queued' where ?1='representation'".into()),
        (IndexSql::RepositoryLatestJob1, "select 'job','representation','concept','concept','queued',1,null,'created',null where ?1='record'".into()),
    ]));
    let mut db = Connection::open_in_memory().unwrap();
    let initial = Cache::default();
    let run = IndexRun::new(&resources, &initial);
    let Error::Need(need) = crate::runtime_plugins::literature_index::repository::queue_retry(
        &run,
        &mut db,
        "representation",
    )
    .unwrap_err() else {
        panic!("finite retry decision");
    };
    let IndexInputV1::Retry {
        latest: Some(summary),
        ..
    } = need.input.clone()
    else {
        panic!("original latest job");
    };
    for field in ["job", "representation", "subject", "status", "attempt"] {
        let mut replacement = summary.clone();
        match field {
            "job" => replacement.job_id = "outside-job".into(),
            "representation" => replacement.representation_id = "outside-representation".into(),
            "subject" => replacement.subject_id = "outside-subject".into(),
            "status" => replacement.status = "succeeded".into(),
            _ => replacement.attempt = 2,
        }
        let cache = Cache(vec![Decided {
            need: (*need).clone(),
            decision: IndexDecisionV1::ReuseJob {
                summary: replacement,
                should_start: true,
            },
            origin: DecisionOrigin {
                package_key: "synthetic-package".into(),
                instance_id: "synthetic-instance".into(),
                native_request_id: "synthetic-request".into(),
            },
        }]);
        let run = IndexRun::new(&resources, &cache);
        let error = crate::runtime_plugins::literature_index::repository::queue_retry(
            &run,
            &mut db,
            "representation",
        )
        .unwrap_err();
        assert!(!error.needs_recovery());
        assert_eq!(
            error.into_wire("retry", Some("representation"))["code"],
            "index_error"
        );
        assert!(db.is_autocommit());
        assert_eq!(db.total_changes(), 0);
    }
}
