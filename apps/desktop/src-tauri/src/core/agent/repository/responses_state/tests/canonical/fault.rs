use super::*;
use crate::data_interface::Database;
use research_agent_core::model::ContextPersistenceDiagnostic;

fn fixture() -> (
    tempfile::TempDir,
    ResponsesStateScope,
    Arc<Database>,
    Vec<Value>,
    CapturedResponsesInput,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("responses.sqlite");
    let scope = initialize_through(&path, 207);
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_initial_context_windows begin insert into fixture_child values(1); end;").unwrap();
    let user = super::super::inheritance::item(&mut sql, &scope, ItemKind::UserMessage, "read");
    let (input, mut captured) = capture(&request(
        &sql,
        &scope,
        INITIAL_ATTEMPT,
        vec![CapturedTurnRef {
            turn_id: scope.turn_id,
            user_item_ids: vec![user],
        }],
    ));
    captured.sampling = true;
    let db = Database::open(sql, &dir.path().join("log")).unwrap();
    (dir, scope, db, input, captured)
}

#[tokio::test]
async fn canonical_identity_commit_failure_fences_owner_until_reopen() {
    let (dir, scope, db, input, captured) = fixture();
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
    let result = state
        .prepare_canonical_input(Uuid::from_u128(1), scope.model.clone(), input, captured)
        .await;
    assert!(
        matches!(result, Err(ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id, phase: "commit", safe_cause: "sql_commit_outcome_unknown", sqlite_extended_code: Some(787),
    })) if conversation_id == scope.conversation_id)
    );
    assert!(db.lock().is_err());
    drop(state);
    drop(db);
    let sql = Connection::open(dir.path().join("responses.sqlite")).unwrap();
    let reopened = Database::open(sql, &dir.path().join("log")).unwrap();
    let guard = reopened.lock().unwrap();
    let count: u64 = guard
        .query_row(
            "select count(*) from agent_initial_context_windows",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn canonical_rollback_failure_fences_owner_but_known_rejection_does_not() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let (_dir, scope, db, input, mut captured) = fixture();
    captured.request_id = Uuid::new_v4();
    let state = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
    let reject = || {
        state.prepare_canonical_input(
            Uuid::from_u128(1),
            scope.model.clone(),
            input.clone(),
            captured.clone(),
        )
    };
    assert!(matches!(reject().await, Err(ModelError::InvalidRequest)));
    {
        let guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    assert!(
        matches!(reject().await, Err(ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id, phase: "rollback", safe_cause: "sql_rollback_failed", ..
    })) if conversation_id == scope.conversation_id)
    );
    assert!(db.lock().is_err());
}
