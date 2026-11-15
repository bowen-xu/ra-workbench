use super::*;
use crate::ai::structured_resource_validation::{Failure, FailureCause, Flow, StructuredPolicy};
use boundary::{BoundaryError, HbsNeed, HbsOutcome, HbsSources};
#[path = "kernel_tests/fixture.rs"]
mod fixture;
use fixture::*;
#[path = "kernel_tests/recovery.rs"]
mod recovery;

#[test]
fn source_requirement_is_after_receipt_and_cas_but_before_null_preservation() {
    let mut db = database();
    let (id, before) = seeded(&mut db);
    let mut sources = Sources::new();
    sources.ready = false;
    let nulls = json!({"format":1,"kind":"structured","observed":{"version":0,"saved":null,"baseline":null},"content":null,"ancestor":null,"reviewAncestor":null,"baselineAncestor":null,"ordinary":[]});
    let changes = db.total_changes();
    let error = preservation::save_with(&mut sources, &mut db, "structured", &id, nulls.clone())
        .unwrap_err();
    let BoundaryError::Pause(p) = error else {
        panic!("sources pause")
    };
    assert_eq!(p.outcome(), HbsOutcome::RolledBack);
    assert!(matches!(p.need(), HbsNeed::Sources));
    assert!(db.is_autocommit());
    assert_eq!(db.total_changes(), changes);
    assert_eq!(sources.policy.calls, 0);
    sources.ready = true;
    preservation::save_with(&mut sources, &mut db, "structured", &id, nulls).unwrap();
    assert_eq!(sources.policy.calls, 0);
    let input = request(&before);
    let result = commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap();
    sources.ready = false;
    let requires = sources.requires;
    let changes = db.total_changes();
    assert_eq!(
        commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap(),
        result
    );
    assert_eq!(sources.requires, requires);
    assert_eq!(db.total_changes(), changes);
    let mut stale = request(&before);
    stale.id = "stale".into();
    assert_eq!(
        commit_with(&mut sources, &mut db, "structured", &id, &stale).unwrap()["type"],
        "conflict"
    );
    assert_eq!(sources.requires, requires);
    let mut changed = input;
    changed.saved = Value::Null;
    let BoundaryError::Failure(f) =
        commit_with(&mut sources, &mut db, "structured", &id, &changed).unwrap_err()
    else {
        panic!("request identity refusal")
    };
    assert!(f.failure.error.starts_with("MODEL_REQUEST_ID_REUSED"));
    assert_eq!(sources.requires, requires);
}

#[test]
fn later_baseline_pause_has_no_business_write_and_retry_commits_only_once() {
    let mut db = database();
    let (id, before) = seeded(&mut db);
    let input = request(&before);
    let mut sources = Sources::new();
    sources.policy.pause_at = Some(1);
    let changes = db.total_changes();
    let error = commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap_err();
    let BoundaryError::Pause(p) = error else {
        panic!("baseline preparation")
    };
    assert_eq!(p.outcome(), HbsOutcome::RolledBack);
    assert!(matches!(p.need(), HbsNeed::Structured { slot: 1, .. }));
    assert_eq!(sources.policy.calls, 2);
    assert!(db.is_autocommit());
    assert_eq!(db.total_changes(), changes);
    assert_eq!(read(&db, "structured", &id).unwrap(), before);
    assert_eq!(
        db.query_row("select count(*) from hbs_commit_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    sources.policy.calls = 0;
    sources.policy.pause_at = None;
    let result = commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap();
    assert_eq!(
        result["snapshot"]["version"].as_i64(),
        Some(input.expected_version + 1)
    );
    let changes = db.total_changes();
    sources.ready = false;
    assert_eq!(
        commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap(),
        result
    );
    assert_eq!(db.total_changes(), changes);
    assert_eq!(
        db.query_row("select count(*) from hbs_commit_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn native_origin_survives_saved_baseline_retained_and_preservation_wrappers() {
    for (reject_at, prefix, retained) in [
        (0, "MODEL_INVALID_DOCUMENT", false),
        (1, "MODEL_INVALID_BASELINE", false),
        (0, "MODEL_INVALID_DOCUMENT", true),
    ] {
        let mut db = database();
        let (id, before) = seeded(&mut db);
        let mut input = request(&before);
        if retained {
            input.saved["retainedFields"] = json!([{"field":"old","status":"filled","statements":[{"id":"r","text":"retained","epistemicRole":"uncertain","evidenceItemIds":[]}]}]);
        }
        let mut sources = Sources::new();
        sources.policy.reject_at = Some(reject_at);
        let changes = db.total_changes();
        assert_rejected(
            commit_with(&mut sources, &mut db, "structured", &id, &input).unwrap_err(),
            prefix,
        );
        assert_eq!(db.total_changes(), changes);
        assert_eq!(read(&db, "structured", &id).unwrap(), before);
    }
    let mut db = database();
    let (id, before) = seeded(&mut db);
    let mut sources = Sources::new();
    sources.policy.reject_at = Some(0);
    let changes = db.total_changes();
    assert_rejected(
        preservation::save_with(
            &mut sources,
            &mut db,
            "structured",
            &id,
            checkpoint(&before, before["saved"].clone()),
        )
        .unwrap_err(),
        "MODEL_RECOVERY_INVALID",
    );
    assert_eq!(db.total_changes(), changes);
}

#[test]
fn actual_none_source_sql_failure_keeps_technical_cause_before_pure_policy() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let mut db = database();
    let (id, before) = seeded(&mut db);
    let mut sources = Sources::new();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "paper_context_snapshots",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let BoundaryError::Failure(f) =
        commit_with(&mut sources, &mut db, "structured", &id, &request(&before)).unwrap_err()
    else {
        panic!("actual SQL source failure")
    };
    assert!(f.failure.error.contains("无法读取文献来源"));
    assert!(matches!(
        f.failure.cause,
        FailureCause::SourceReadUnavailable
    ));
    assert_eq!(f.outcome, HbsOutcome::RolledBack);
    assert_eq!(sources.policy.calls, 0);
    assert!(db.is_autocommit());
}
