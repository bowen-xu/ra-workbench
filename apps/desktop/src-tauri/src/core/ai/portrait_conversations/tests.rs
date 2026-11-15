use crate::ai::{
    candidate_test_fixture::{TestDirectory, database, insert_selection_source, selection_draft},
    error::AiError,
    model::{AiRun, AiTaskSpecDraft},
    run_repository,
};
use rusqlite::{Connection, params};
use uuid::Uuid;
fn task() -> AiTaskSpecDraft {
    selection_draft("anchor-ai")
}
fn fixture() -> (Connection, TestDirectory) {
    let db = database();
    insert_selection_source(&db);
    (db, TestDirectory::new())
}
fn start(db: &mut Connection, directory: &TestDirectory) -> Result<AiRun, AiError> {
    let mut draft = task();
    draft.confirmed_scope_hash = Some(
        crate::ai::candidate_test_fixture::preview(db, &directory.credentials(), &draft)?
            .scope_hash,
    );
    run_repository::insert_run(db, &directory.credentials(), &draft)
}
#[test]
fn shared_turn_lookup_distinguishes_missing_from_database_failure() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (mut db, directory) = fixture();
    let run = start(&mut db, &directory).unwrap();
    assert!(
        run_repository::get_run(&db, &run.id)
            .unwrap()
            .shared_turn_id
            .is_none()
    );

    let denied = Arc::new(AtomicBool::new(false));
    let hit = denied.clone();
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "agent_submissions",
            ..
        } => {
            hit.store(true, Ordering::SeqCst);
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }));
    let result = run_repository::get_run(&db, &run.id);
    crate::ai::legacy_binding_fault_tests::clear_authorizer(&db);
    assert!(denied.load(Ordering::SeqCst));
    assert!(matches!(result, Err(AiError::Database(_))));

    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,'invalid-uuid',?3,1,'{}')",
        params![run.shared_conversation_id.unwrap(), run.shared_request_id.unwrap(), Uuid::new_v4().to_string()]).unwrap();
    assert!(matches!(
        run_repository::get_run(&db, &run.id),
        Err(AiError::Database(_))
    ));
}

#[test]
fn shared_turn_lookup_failure_before_commit_does_not_publish_run() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (mut db, directory) = fixture();
    let mut draft = task();
    draft.confirmed_scope_hash = Some(
        crate::ai::candidate_test_fixture::preview(&db, &directory.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let denied = Arc::new(AtomicBool::new(false));
    let hit = denied.clone();
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "agent_submissions",
            ..
        } => {
            hit.store(true, Ordering::SeqCst);
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }));
    let result = run_repository::insert_run(&mut db, &directory.credentials(), &draft);
    crate::ai::legacy_binding_fault_tests::clear_authorizer(&db);
    assert!(denied.load(Ordering::SeqCst));
    assert!(matches!(result, Err(AiError::Database(_))));
    for table in [
        "ai_runs",
        "ai_scope_items",
        "agent_conversations",
        "agent_legacy_ai_run_bindings",
    ] {
        let count: i64 = db
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} must not be published");
    }
}
