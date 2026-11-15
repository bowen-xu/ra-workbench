use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

mod equality;
pub(crate) mod fixture;
mod rollback;
mod transactions;
use fixture::{Fixture, OPERATION, classification, snapshot};

fn nested_table(name: &str) -> bool {
    name.starts_with("collection") || name == "item_relation_assignments" || nal_table(name)
}

fn nal_table(name: &str) -> bool {
    name.starts_with("nal_") || name == "concepts"
}

fn clear_authorizer(fixture: &Fixture) {
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

fn revisions(fixture: &Fixture) -> (i64, i64) {
    let db = fixture.data.db.lock().unwrap();
    (
        db.query_row("select revision from collection_state", [], |r| r.get(0))
            .unwrap(),
        db.query_row("select revision from nal_state", [], |r| r.get(0))
            .unwrap(),
    )
}

#[test]
fn missing_header_and_original_bad_snapshot_fail_before_begin() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.catalog.discover().unwrap().packages[0].package_key,
        fixture.key
    );
    fixture.seed_raw(r#"{"createdReadingState":"not-a-boolean"}"#);
    let begins = Arc::new(AtomicUsize::new(0));
    let count = begins.clone();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Begin
                }
            ) {
                count.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let missing = capture(
        &fixture.data.db.lock().unwrap(),
        &fixture.resources,
        "missing".into(),
    )
    .err()
    .unwrap();
    assert!(matches!(missing.phase, UndoPhase::Header));
    assert!(matches!(missing.cause, UndoCause::Missing));
    assert_eq!(missing.code(), "not_found");
    let invalid = capture(
        &fixture.data.db.lock().unwrap(),
        &fixture.resources,
        OPERATION.into(),
    )
    .err()
    .unwrap();
    assert!(matches!(invalid.phase, UndoPhase::Snapshot));
    assert!(matches!(invalid.cause, UndoCause::SnapshotCodec(_)));
    assert_eq!(invalid.code(), "connector_error");
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    clear_authorizer(&fixture);
}

#[test]
fn absent_classification_skips_every_nested_read_and_both_revision_bumps() {
    let fixture = Fixture::new();
    let before = revisions(&fixture);
    let note = fixture.data.note();
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read { table_name, .. } if nested_table(table_name) => {
                count.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
    fixture.apply().unwrap();
    clear_authorizer(&fixture);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(revisions(&fixture), before);
    assert_eq!(fixture.data.note(), note);
    let after = fixture.state();
    assert_eq!(fixture.reopen().state(), after);
}

#[test]
fn absent_nal_still_checks_classification_without_reading_nal() {
    let fixture = Fixture::new();
    let before = revisions(&fixture);
    let mut value = snapshot();
    value["classification"] = classification();
    fixture.seed_snapshot(&value);
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read { table_name, .. } if nal_table(table_name) => {
                count.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
    fixture.apply().unwrap();
    clear_authorizer(&fixture);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(revisions(&fixture), (before.0 + 1, before.1));
}

#[test]
fn classification_conflict_precedes_unrelated_malformed_nal_without_nal_reads() {
    let fixture = Fixture::new();
    let mut value = snapshot();
    value["classification"] = classification();
    value["classification"]["nal"] = json!({"before":[],"after":[],
        "before_annotations":[],"after_annotations":[]});
    fixture.seed_snapshot(&value);
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "insert into collections(id,name,description) values('later','Later','');
         insert into collection_memberships(item_id,collection_id) values('survivor','later');
         insert into nal_statements(id,subject,predicate,origin)
         values('bad','{\"kind\":\"variable\",\"name\":\"bad-fact\"}',
         '{\"kind\":\"paper\",\"id\":\"source\"}','human');",
        )
        .unwrap();
    let before = fixture.state();
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read { table_name, .. } if nal_table(table_name) => {
                count.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
    let failure = fixture.apply().unwrap_err();
    clear_authorizer(&fixture);
    assert!(matches!(
        failure.problem.phase,
        UndoPhase::ClassificationCheck
    ));
    assert!(matches!(failure.problem.cause, UndoCause::Conflict));
    assert_eq!(failure.outcome, UndoOutcome::RolledBack);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn captured_header_is_not_requeried_or_rejected_by_a_new_stale_rule() {
    let fixture = Fixture::new();
    let captured = fixture.capture();
    let plan = fixture.prepared(&captured);
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute(
            "update merge_operations set state='undone',snapshot_json='{}' where id=?1",
            [OPERATION],
        )
        .unwrap();
    apply(
        &mut fixture.data.db.lock().unwrap(),
        &fixture.resources,
        &captured,
        &plan,
    )
    .unwrap();
    let db = fixture.data.db.lock().unwrap();
    let stored: String = db
        .query_row(
            "select snapshot_json from merge_operations where id=?1",
            [OPERATION],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "{}");
    let decision: Option<String> = db
        .query_row(
            "select decision from duplicate_candidates where id='candidate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(decision.is_none());
}
