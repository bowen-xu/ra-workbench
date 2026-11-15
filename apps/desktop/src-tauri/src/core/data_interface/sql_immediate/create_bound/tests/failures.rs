use super::*;

#[test]
fn a_failed_binding_insert_rolls_back_the_new_conversation_and_keeps_no_proof() {
    let (_dir, db, command) = fixture();
    db.lock().unwrap().execute_batch("create trigger fail_create_binding before insert on agent_conversation_bindings begin select raise(abort,'synthetic binding failure'); end;").unwrap();
    let error = db.create_bound_conversation(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "mutate");
    assert_eq!(
        error.diagnostic().unwrap().operation_id,
        command.operation_id
    );
    assert_eq!(records(&db.raw.lock().unwrap()), (0, 0, 0));
    db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_create_binding;")
        .unwrap();
    assert!(!db.create_bound_conversation(&command).unwrap().replayed);
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 1, 1));
}

#[test]
fn a_failed_proof_insert_rolls_back_both_business_rows_and_allows_the_same_intent() {
    let (_dir, db, command) = fixture();
    db.lock().unwrap().execute_batch("create trigger fail_create_proof before insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound' begin select raise(abort,'synthetic proof failure'); end;").unwrap();
    let error = db.create_bound_conversation(&command).unwrap_err();
    assert_eq!(error.code(), "DATA_SQL_UNAVAILABLE");
    assert_eq!(error.diagnostic().unwrap().phase, "receipt");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert_eq!(records(&db.raw.lock().unwrap()), (0, 0, 0));
    assert!(matches!(
        db.bound_conversation_creation_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_create_proof;")
        .unwrap();
    assert!(!db.create_bound_conversation(&command).unwrap().replayed);
}

#[test]
fn unknown_before_commit_does_not_authorize_recreation_until_the_owner_reopens() {
    let (dir, db, command) = fixture();
    let error = db
        .create_bound_conversation_owned(&command, Fault::BeforeCommit)
        .unwrap_err();
    assert_eq!(error.code(), "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        error.diagnostic().unwrap().operation_id,
        command.operation_id
    );
    assert!(matches!(
        db.bound_conversation_creation_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.create_bound_conversation(&command).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    assert_eq!(records(&observer), (0, 0, 0));
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(matches!(
        reopened
            .bound_conversation_creation_status(&command)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(
        !reopened
            .create_bound_conversation(&command)
            .unwrap()
            .replayed
    );
    assert_eq!(records(&reopened.raw.lock().unwrap()), (1, 1, 1));
}

#[test]
fn lost_commit_reply_is_readable_as_original_proof_without_unblocking_or_reapplying() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.create_bound_conversation_owned(&command, Fault::AfterCommit),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    let proof = match db.bound_conversation_creation_status(&command).unwrap() {
        SqlImmediateStatus::Committed(value) => value,
        _ => panic!("expected original committed proof"),
    };
    assert_eq!(proof.operation_id, command.operation_id);
    assert_eq!(proof.sql_fence.position, 0);
    assert!(db.create_bound_conversation(&command).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    let original = metadata(&observer, command.id);
    assert_eq!(records(&observer), (1, 1, 1));
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert_eq!(reopened.create_bound_conversation(&command).unwrap(), proof);
    assert_eq!(
        metadata(&reopened.raw.lock().unwrap(), command.id),
        original
    );
}

#[test]
fn same_connection_uncommitted_rows_and_proof_are_not_a_committed_status() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.create_bound_conversation_owned(&command, Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 1, 1));
    assert!(matches!(
        db.bound_conversation_creation_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed {
            reason: "sql_transaction_open",
            ..
        }
    ));
    assert!(!db.raw.lock().unwrap().is_autocommit());
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    assert_eq!(records(&observer), (0, 0, 0));
    // Test cleanup only: status itself did not clean, commit or repair anything.
    db.raw.lock().unwrap().execute_batch("rollback;").unwrap();
    assert!(db.lock().is_err());
    drop(observer);
    drop(db);
    assert!(matches!(
        open(dir.path())
            .bound_conversation_creation_status(&command)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
}

#[test]
fn a_real_deferred_constraint_commit_failure_rolls_back_conversation_binding_and_proof() {
    let (dir, db, command) = fixture();
    db.lock().unwrap().execute_batch("create table create_bound_parent(id integer primary key);
        create table create_bound_child(id integer references create_bound_parent(id) deferrable initially deferred);
        create trigger defer_creation_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound'
        begin insert into create_bound_child(id) values(9); end;").unwrap();
    let error = db.create_bound_conversation(&command).unwrap_err();
    assert_eq!(error.code(), "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert!(db.raw.lock().unwrap().is_autocommit());
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    assert_eq!(records(&observer), (0, 0, 0));
    assert_eq!(count(&observer, "create_bound_child"), 0);
    assert!(matches!(
        db.bound_conversation_creation_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.create_bound_conversation(&command).is_err());
    // Remove only the synthetic fault using a separate observer connection.
    observer
        .execute_batch("drop trigger defer_creation_commit;")
        .unwrap();
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(matches!(
        reopened
            .bound_conversation_creation_status(&command)
            .unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(
        !reopened
            .create_bound_conversation(&command)
            .unwrap()
            .replayed
    );
}

#[test]
fn corrupt_own_proof_is_observed_without_repair_and_blocks_a_mutation_attempt() {
    let (_dir, db, command) = fixture();
    db.create_bound_conversation(&command).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set result_json='1' where operation_id=?1",
            [command.operation_id.to_string()],
        )
        .unwrap();
    let before = metadata(&db.raw.lock().unwrap(), command.id);
    assert!(matches!(
        db.bound_conversation_creation_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(db.lock().is_ok());
    assert!(matches!(
        db.create_bound_conversation(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(db.lock().is_err());
    assert_eq!(metadata(&db.raw.lock().unwrap(), command.id), before);
    assert_eq!(
        db.raw
            .lock()
            .unwrap()
            .query_row(
                "select result_json from data_sql_operation_receipts",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "1"
    );
}

#[test]
fn invalid_original_identities_never_create_business_rows_or_receipts() {
    let (_dir, db, command) = fixture();
    for changed in [
        CreateBoundConversation {
            operation_id: Uuid::nil(),
            ..command.clone()
        },
        CreateBoundConversation {
            id: ConversationId::from_uuid(Uuid::nil()),
            ..command.clone()
        },
        CreateBoundConversation {
            binding: ConversationBinding {
                resource_id: "\0".into(),
                ..command.binding.clone()
            },
            ..command.clone()
        },
    ] {
        assert!(db.create_bound_conversation(&changed).is_err());
        assert!(db.bound_conversation_creation_status(&changed).is_err());
    }
    assert_eq!(records(&db.raw.lock().unwrap()), (0, 0, 0));
}
