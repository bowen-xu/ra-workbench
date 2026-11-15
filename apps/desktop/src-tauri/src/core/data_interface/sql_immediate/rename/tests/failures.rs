use super::*;

#[test]
fn rejected_receipt_insert_rolls_back_the_title_and_original_intent_can_retry() {
    let (_dir, db, command) = fixture();
    let before = metadata(&db.raw.lock().unwrap(), &command);
    db.lock().unwrap().execute_batch("create trigger reject_rename_receipt before insert on data_sql_operation_receipts when new.kind='agent.conversation.rename' begin select raise(abort,'synthetic receipt failure'); end;").unwrap();
    let error = db.rename_conversation(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "receipt");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    {
        let sql = db.lock().unwrap();
        assert!(sql.is_autocommit());
        assert_eq!(metadata(&sql, &command), before);
        assert_eq!(receipt_count(&sql), 0);
        sql.execute_batch("drop trigger reject_rename_receipt;")
            .unwrap();
    }
    assert!(!db.rename_conversation(&command).unwrap().replayed);
    assert_eq!(metadata(&db.raw.lock().unwrap(), &command).0, "B");
}

#[test]
fn commit_unknown_before_commit_stays_unconfirmed_until_reopen() {
    let (dir, db, command) = fixture();
    let error = db
        .rename_conversation_owned(&command, Fault::BeforeCommit)
        .unwrap_err();
    assert_eq!(error.code(), "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        error.diagnostic().unwrap().operation_id,
        command.operation_id
    );
    assert!(matches!(
        db.rename_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.rename_conversation(&command).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("rename.sqlite3")).unwrap();
    assert_eq!(metadata(&observer, &command).0, "A");
    assert_eq!(receipt_count(&observer), 0);
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(matches!(
        reopened.rename_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(!reopened.rename_conversation(&command).unwrap().replayed);
    assert_eq!(metadata(&reopened.raw.lock().unwrap(), &command).0, "B");
}

#[test]
fn committed_but_lost_reply_is_observable_without_unblocking_or_reapplying() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.rename_conversation_owned(&command, Fault::AfterCommit),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    let proof = match db.rename_status(&command).unwrap() {
        SqlImmediateStatus::Committed(value) => value,
        _ => panic!("committed proof must be readable"),
    };
    assert_eq!(proof.operation_id, command.operation_id);
    assert_eq!(proof.sql_fence.position, 0);
    assert!(db.rename_conversation(&command).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("rename.sqlite3")).unwrap();
    assert_eq!(metadata(&observer, &command).0, "B");
    assert_eq!(receipt_count(&observer), 1);
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert_eq!(reopened.rename_conversation(&command).unwrap(), proof);
    assert_eq!(receipt_count(&reopened.raw.lock().unwrap()), 1);
}

#[test]
fn same_connection_uncommitted_title_and_receipt_are_not_a_committed_status() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.rename_conversation_owned(&command, Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    {
        let sql = db.raw.lock().unwrap();
        assert!(!sql.is_autocommit());
        assert_eq!(metadata(&sql, &command).0, "B");
        assert_eq!(receipt_count(&sql), 1);
    }
    let observer = Connection::open(dir.path().join("rename.sqlite3")).unwrap();
    assert_eq!(metadata(&observer, &command).0, "A");
    assert_eq!(receipt_count(&observer), 0);
    for _ in 0..2 {
        assert!(matches!(
            db.rename_status(&command).unwrap(),
            SqlImmediateStatus::Unconfirmed {
                reason: "sql_transaction_open",
                ..
            }
        ));
        assert!(!db.raw.lock().unwrap().is_autocommit());
    }
    // Only explicit test cleanup rolls back. Observing never repairs an owner.
    db.raw.lock().unwrap().execute_batch("rollback;").unwrap();
    assert!(matches!(
        db.rename_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.lock().is_err());
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(!reopened.rename_conversation(&command).unwrap().replayed);
}

#[test]
fn a_real_deferred_constraint_commit_failure_rolls_back_both_rows_and_blocks() {
    let (dir, db, command) = fixture();
    db.lock().unwrap().execute_batch("create table rename_parent(id text primary key);
      create table rename_child(id text primary key,parent_id text references rename_parent(id) deferrable initially deferred);
      create trigger reject_rename_commit after update of title on agent_conversations begin insert into rename_child values(new.id,'missing'); end;").unwrap();
    let error = db.rename_conversation(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert!(db.raw.lock().unwrap().is_autocommit());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("rename.sqlite3")).unwrap();
    assert_eq!(metadata(&observer, &command).0, "A");
    assert_eq!(receipt_count(&observer), 0);
    assert!(matches!(
        db.rename_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
}

#[test]
fn corrupt_own_result_blocks_a_write_but_observation_does_not_repair_it() {
    let (_dir, db, command) = fixture();
    db.rename_conversation(&command).unwrap();
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set result_json='[]'",
            [],
        )
        .unwrap();
    assert!(matches!(
        db.rename_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(
        !db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .blocked
    );
    assert!(matches!(
        db.rename_conversation(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(db.lock().is_err());
    assert!(matches!(
        db.rename_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert_eq!(metadata(&db.raw.lock().unwrap(), &command).0, "B");
}

#[test]
fn closed_owner_keeps_original_proof_and_does_not_admit_a_new_operation() {
    let (_dir, db, command) = fixture();
    let proof = db.rename_conversation(&command).unwrap();
    db.shutdown().unwrap();
    assert!(
        matches!(db.rename_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    let unknown = RenameConversation {
        operation_id: Uuid::new_v4(),
        ..command.clone()
    };
    assert!(matches!(
        db.rename_status(&unknown).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { closed: true, .. }
    ));
    assert!(matches!(
        db.rename_conversation(&unknown),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
}

#[test]
fn status_rejects_wrong_epoch_or_future_receipt_without_mutating_the_owner() {
    for corrupt in ["epoch", "future"] {
        let (_dir, db, command) = fixture();
        db.rename_conversation(&command).unwrap();
        {
            let sql = db.raw.lock().unwrap();
            if corrupt == "epoch" {
                sql.execute(
                    "update data_runtime_state set epoch=?1",
                    [Uuid::new_v4().to_string()],
                )
                .unwrap();
            } else {
                sql.execute(
                    "update data_sql_operation_receipts set sql_fence_position=1",
                    [],
                )
                .unwrap();
            }
        }
        let before = db
            .owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot();
        assert!(matches!(
            db.rename_status(&command),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert_eq!(
            db.owner
                .as_ref()
                .unwrap()
                .inspection_guard()
                .unwrap()
                .snapshot(),
            before
        );
    }
}
