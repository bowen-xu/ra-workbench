use super::*;

#[test]
fn handler_completion_and_context_commit_atomically_and_require_all_identities() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("handlers.sqlite");
    let (mut db, request) = setup(&path);
    let start = db.last_insert_rowid();
    let id = Uuid::new_v4();
    let handlers = [(id, Some(CompactionHookOutcome::Stop))];
    super::super::super::handlers::record(&db, start, &[(id, None)]).unwrap();
    // Omitting a configured command must not commit aggregate completion.
    assert!(record_result(&mut db, &request, &result(), 2).is_err());
    db.execute_batch("create trigger fail_handler before insert on agent_compaction_hook_handlers when new.outcome is not null begin select raise(abort,'fixture'); end;").unwrap();
    assert!(record_result_with_handlers(&mut db, &request, &result(), 2, &handlers).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compaction_hook_audit where notification='completed'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compact_hook_contexts",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_handler").unwrap();
    assert!(
        record_result_with_handlers(
            &mut db,
            &request,
            &result(),
            2,
            &[(Uuid::new_v4(), Some(CompactionHookOutcome::Stop))]
        )
        .is_err()
    );
    record_result_with_handlers(&mut db, &request, &result(), 3, &handlers).unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        read(&db, request.conversation_id, request.operation_id, 0, 1).unwrap()[0].contexts,
        result().additional_contexts
    );
    let states: Vec<Option<String>> = db
        .prepare("select outcome from agent_compaction_hook_handlers order by audit_seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(states, [None, Some("stop".into())]);
    assert!(
        db.execute(
            "update agent_compaction_hook_handlers set outcome='failed'",
            []
        )
        .is_err()
    );
}

#[test]
fn handler_start_failure_rolls_back_aggregate_admission() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, mut request) = setup(&root.path().join("handlers.sqlite"));
    request.operation_id = Uuid::new_v4();
    db.execute_batch("create trigger fail_handler before insert on agent_compaction_hook_handlers begin select raise(abort,'fixture'); end;").unwrap();
    {
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::super::super::record(&tx, &request, CompactionHookNotification::Started, 2).unwrap();
        assert!(
            super::super::super::handlers::record(
                &tx,
                tx.last_insert_rowid(),
                &[(Uuid::new_v4(), None)]
            )
            .is_err()
        );
        // Deliberately do not commit failed admission.
    }
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compaction_hook_audit",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
