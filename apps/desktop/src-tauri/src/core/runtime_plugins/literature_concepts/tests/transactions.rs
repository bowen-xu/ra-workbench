use super::*;
use rusqlite::hooks::TransactionOperation;

#[test]
fn failed_rollback_retains_original_cause_fences_owner_and_cold_reopen_rolls_back() {
    let f = Fixture::new();
    let note = f.note();
    let before = f.state();
    let mut db = f.db.lock().unwrap();
    db.execute_batch("create trigger concept_test_insert_abort before insert on concepts begin select raise(abort,'synthetic concept insert abort'); end;").unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = match apply(&mut db, &f.resources, plan(draft())) {
        Err(e) => e,
        _ => panic!("write failure"),
    };
    assert_eq!(error.outcome, ConceptOutcome::Unknown);
    assert_eq!(error.phase, "concept-insert");
    assert!(error.concept_id.is_some()); // UUID was actually allocated before this INSERT.
    let ConceptProblem::Data(original) = error.problem.as_ref() else {
        panic!("original write cause")
    };
    assert_eq!(original.sqlite_extended_code, Some(1811));
    assert_eq!(error.rollback_error.as_ref().unwrap().phase, "rollback");
    assert!(!db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    assert!(f.db.lock().is_err());
    let f = f.reopen();
    assert_eq!(f.state(), before);
    assert_eq!(f.note(), note);
}

#[test]
fn commit_hook_rollback_without_explicit_receipt_remains_unknown_and_fenced() {
    let f = Fixture::new();
    let note = f.note();
    let before = f.state();
    let mut db = f.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let error = match apply(&mut db, &f.resources, plan(draft())) {
        Err(e) => e,
        _ => panic!("commit hook failure"),
    };
    assert_eq!(error.phase, "commit");
    assert_eq!(error.outcome, ConceptOutcome::Unknown);
    assert!(error.concept_id.is_some());
    assert!(error.rollback_error.is_none());
    assert!(db.is_autocommit());
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(f.db.lock().is_err());
    let f = f.reopen();
    assert_eq!(f.state(), before);
    assert_eq!(f.note(), note);
}

#[test]
fn denied_commit_with_successful_explicit_rollback_keeps_owner_usable() {
    let f = Fixture::new();
    let before = f.state();
    let mut db = f.db.lock().unwrap();
    // rusqlite 0.37 exposes COMMIT as Unknown; no accepted concept resource
    // contains transaction control, so this identifies the explicit final COMMIT.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = match apply(&mut db, &f.resources, plan(draft())) {
        Err(e) => e,
        _ => panic!("COMMIT denied"),
    };
    assert_eq!(error.phase, "commit");
    assert_eq!(error.outcome, ConceptOutcome::RolledBack);
    assert!(error.rollback_error.is_none());
    assert!(db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    assert_eq!(f.state(), before);
    assert_eq!(f.reopen().state(), before);
}
