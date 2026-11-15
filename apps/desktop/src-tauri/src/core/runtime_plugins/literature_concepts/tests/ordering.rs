use super::*;
use research_workbench_native_sdk::concepts::ConceptRejectionV1 as R;
use rusqlite::{functions::FunctionFlags, hooks::TransactionOperation};
use std::sync::atomic::AtomicUsize;

#[test]
fn name_collision_precedes_term_rejection_and_nested_begin_precedes_both() {
    let f = Fixture::new();
    f.save(plan(draft())).unwrap();
    let before = f.state();
    let mut value = plan(draft());
    parts(&mut value).0.rejection = Some(R::EmptyTerm);
    let error = f.save(value.clone()).unwrap_err();
    assert_eq!(error.phase, "concept-name-available");
    assert!(matches!(
        error.problem.as_ref(),
        ConceptProblem::Domain(R::NameUnavailable)
    ));
    assert_eq!(error.outcome, ConceptOutcome::RolledBack);
    assert_eq!(f.state(), before);
    let mut db = f.db.lock().unwrap();
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let error = match apply(&mut db, &f.resources, value) {
        Err(e) => e,
        _ => panic!("nested begin"),
    };
    assert_eq!(error.phase, "begin");
    assert_eq!(error.outcome, ConceptOutcome::NotStarted);
    assert!(!db.is_autocommit());
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert_eq!(f.state(), before);
}

#[test]
fn actual_term_insert_failure_precedes_cas_and_new_expected_revision() {
    let f = Fixture::new();
    let original = f.save(plan(draft())).unwrap();
    let before = f.state();
    f.db.lock().unwrap().execute_batch("create trigger concept_test_term_failure before insert on terms begin select raise(abort,'synthetic term insert failure'); end;").unwrap();
    for existing in [false, true] {
        let mut input = draft();
        input.label = "A distinct name".into();
        input.expected_revision = Some(i64::MIN);
        if existing {
            input.id = Some(original.id.clone());
        }
        let mut value = plan(input);
        let (term, rules) = parts(&mut value);
        term.normalized_text = "new unseen synthetic term".into();
        if !existing {
            rules.new_expected_rejection = Some(R::NewExpectedRevision);
        }
        let error = f.save(value).unwrap_err();
        assert_eq!(error.phase, "term-insert");
        assert_eq!(error.concept_id, existing.then(|| original.id.clone()));
        assert_eq!(error.outcome, ConceptOutcome::RolledBack);
        let ConceptProblem::Data(cause) = error.problem.as_ref() else {
            panic!("actual term SQL error")
        };
        assert_eq!(cause.sqlite_extended_code, Some(1811));
        assert_eq!(f.state(), before);
    }
}

#[test]
fn successful_term_write_precedes_delayed_new_expected_refusal_and_is_rolled_back() {
    let f = Fixture::new();
    let before = f.state();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    f.db.lock()
        .unwrap()
        .create_scalar_function(
            "observe_concept_term",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(0_i64)
            },
        )
        .unwrap();
    f.db.lock().unwrap().execute_batch("create trigger concept_test_term_observer after insert on terms begin select observe_concept_term(); end;").unwrap();
    let mut input = draft();
    input.expected_revision = Some(-1);
    let mut value = plan(input);
    parts(&mut value).1.new_expected_rejection = Some(R::NewExpectedRevision);
    let error = f.save(value).unwrap_err();
    assert!(matches!(
        error.problem.as_ref(),
        ConceptProblem::Domain(R::NewExpectedRevision)
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(f.state(), before);
    assert_eq!(f.reopen().state(), before);
}

#[test]
fn second_create_name_check_observes_term_trigger_and_rolls_back_its_changes() {
    let f = Fixture::new();
    let existing = f.save(plan(draft())).unwrap();
    let before = f.state();
    f.db.lock().unwrap().execute_batch(&format!(
        "create trigger concept_test_late_name after insert on terms begin update concepts set label='Late collision' where id='{}'; end;",
        existing.id,
    )).unwrap();
    let mut input = draft();
    input.label = "Late collision".into();
    let mut value = plan(input);
    parts(&mut value).0.normalized_text = "unseen late collision term".into();
    let error = f.save(value).unwrap_err();
    assert_eq!(error.phase, "concept-name-available");
    assert!(matches!(
        error.problem.as_ref(),
        ConceptProblem::Domain(R::NameUnavailable)
    ));
    assert_eq!(f.state(), before);
    assert_eq!(f.reopen().state(), before);
}

#[test]
fn begin_failure_is_not_replaced_by_deferred_term_validation() {
    let f = Fixture::new();
    let mut value = plan(draft());
    parts(&mut value).0.rejection = Some(R::EmptyTerm);
    let mut db = f.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Begin,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = match apply(&mut db, &f.resources, value) {
        Err(e) => e,
        _ => panic!("BEGIN denied"),
    };
    assert_eq!(error.phase, "begin");
    assert_eq!(error.outcome, ConceptOutcome::NotStarted);
    assert!(db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}
