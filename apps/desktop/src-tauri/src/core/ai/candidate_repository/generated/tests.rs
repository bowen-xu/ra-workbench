use super::*;
use crate::{
    ai::candidate_test_fixture::{insert_selection_source, prepare, seed, selection_payload},
    ai::generation_policy::PreparedCandidate,
    data_interface::Database,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::Arc;

fn fixture() -> (
    tempfile::TempDir,
    Arc<Database>,
    String,
    AiEvidenceBundle,
    PreparedCandidate,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let mut connection = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    let (anchor, _) = insert_selection_source(&connection);
    let (run, bundle) = seed(&connection, &anchor);
    let prepared = prepare(
        &bundle,
        &selection_payload("private candidate result").to_string(),
    );
    let owner = Database::open(connection, &root.path().join("log")).unwrap();
    (root, owner, run.id, bundle, prepared)
}

#[test]
fn candidate_commit_failure_blocks_the_original_owner_and_reopens_without_a_candidate() {
    let (root, owner, run, bundle, prepared) = fixture();
    let mut guard = owner.lock().unwrap();
    guard.commit_hook(Some(|| true));
    let error = save_prepared_candidate_guarded(&mut guard, &run, &bundle, prepared).unwrap_err();
    assert_eq!(error.code(), "data_operation_outcome_unknown");
    let message = error.to_string();
    assert!(message.contains(&run));
    assert!(message.contains("commit"));
    assert!(message.contains("storage_error"));
    assert!(!message.contains("private candidate result"));
    drop(guard);
    assert!(owner.lock().is_err());
    drop(owner);
    let reopened = Database::open(
        Connection::open(root.path().join("data.sqlite")).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let guard = reopened.lock().unwrap();
    let count: i64 = guard
        .query_row(
            "select count(*) from ai_candidates where run_id=?1",
            [run],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn candidate_failed_rollback_retains_safe_causes_and_fences_the_original_owner() {
    let (root, owner, run, bundle, prepared) = fixture();
    let mut guard = owner.lock().unwrap();
    guard.authorizer(Some(|context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Insert {
                table_name: "ai_candidate_revisions"
            } | AuthAction::Transaction {
                operation: TransactionOperation::Rollback
            }
        ) {
            Authorization::Deny
        } else {
            Authorization::Allow
        }
    }));
    let error = save_prepared_candidate_guarded(&mut guard, &run, &bundle, prepared).unwrap_err();
    assert_eq!(error.code(), "data_operation_outcome_unknown");
    let message = error.to_string();
    assert!(message.contains(&run));
    assert!(message.contains("rollback"));
    assert!(message.contains("sqlite_operation_failed"));
    assert!(!message.contains("private candidate result"));
    drop(guard);
    assert!(owner.lock().is_err());
    drop(owner);
    let reopened = Database::open(
        Connection::open(root.path().join("data.sqlite")).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let guard = reopened.lock().unwrap();
    let count: i64 = guard
        .query_row(
            "select count(*) from ai_candidates where run_id=?1",
            [run],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
