use super::*;
use crate::runtime_plugins::literature_nal_bootstrap::{
    outcome::{BootstrapCause, BootstrapOutcome},
    prepare::{capture_facts, capture_source},
};
use rusqlite::hooks::TransactionOperation;

#[test]
fn both_short_captures_settle_before_an_original_core_note_write() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    let source = capture_source(&mut db, &fixture.resources).unwrap();
    assert!(db.is_autocommit());
    let (probes, identities) = fixture::empty_preparation();
    let facts = capture_facts(&mut db, &fixture.resources, &source, &probes, &identities).unwrap();
    assert!(db.is_autocommit());
    drop(db);
    assert_eq!(std::sync::Arc::strong_count(&fixture.db), 1);
    // Both prepared objects are still alive while the same Core owner writes a
    // real note in a new caller transaction; neither captures a guard or owner.
    let mut db = fixture.db.lock().unwrap();
    let tx = db.transaction().unwrap();
    let id = crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Original Core note after R1".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(!id.is_empty());
    assert_eq!(source.source, facts.source);
}

#[test]
fn preparation_begin_failure_preserves_the_callers_open_transaction() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.execute_batch("BEGIN DEFERRED;update nal_state set revision=7 where id=1;")
        .unwrap();
    let Err(error) = capture_source(&mut db, &fixture.resources) else {
        panic!("nested BEGIN succeeded")
    };
    assert_eq!(error.outcome, BootstrapOutcome::NotStarted);
    assert!(!db.is_autocommit());
    assert!(matches!(error.problem.cause, BootstrapCause::Database(_)));
    assert_eq!(
        db.query_row("select revision from nal_state where id=1", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        7
    );
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert!(fixture.db.lock().unwrap().is_autocommit());
}

#[test]
fn ignored_read_rollback_is_unknown_and_fences_until_pair_reopen() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let Err(error) = capture_source(&mut db, &fixture.resources) else {
        panic!("ignored rollback acknowledged")
    };
    assert_eq!(
        error.outcome,
        BootstrapOutcome::Unknown {
            observed_revision: None
        }
    );
    assert!(!db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    assert!(fixture.db.lock().is_err());
    let reopened = fixture.reopen_after_recovery_fence();
    assert!(reopened.db.lock().unwrap().is_autocommit());
}

#[test]
fn failed_r1_and_denied_rollback_keep_both_safe_causes() {
    let fixture = Fixture::new();
    fixture.collection();
    let mut db = fixture.db.lock().unwrap();
    let source = capture_source(&mut db, &fixture.resources).unwrap();
    db.execute(
        "update collections set name='new source value' where id='source'",
        [],
    )
    .unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let (probes, identities) = fixture::empty_preparation();
    let Err(error) = capture_facts(&mut db, &fixture.resources, &source, &probes, &identities)
    else {
        panic!("changed source acknowledged")
    };
    assert_eq!(
        error.outcome,
        BootstrapOutcome::Unknown {
            observed_revision: None
        }
    );
    assert!(matches!(
        error.problem.cause,
        BootstrapCause::Capture(CaptureError::SnapshotChanged { .. })
    ));
    let rollback = error.rollback_error.unwrap();
    assert_eq!(rollback.phase, "rollback");
    assert!(rollback.sqlite_extended_code.is_some());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    assert!(fixture.db.lock().is_err());
    let reopened = fixture.reopen_after_recovery_fence();
    assert!(reopened.db.lock().unwrap().is_autocommit());
}
