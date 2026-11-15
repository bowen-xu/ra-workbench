use super::*;

#[tokio::test]
async fn rejected_receipt_insert_rolls_back_the_archived_and_original_intent_can_retry() {
    let (_dir, db, command, prepared) = fixture().await;
    let before = metadata(&db.raw.lock().unwrap(), &command);
    db.lock().unwrap().execute_batch("create trigger reject_archive_receipt before insert on data_sql_operation_receipts when new.kind='agent.conversation.archive' begin select raise(abort,'synthetic receipt failure'); end;").unwrap();
    let error = db.archive_conversation(&prepared).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "receipt");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    {
        let sql = db.lock().unwrap();
        assert!(sql.is_autocommit());
        assert_eq!(metadata(&sql, &command), before);
        assert_eq!(receipt_count(&sql), 0);
        sql.execute_batch("drop trigger reject_archive_receipt;")
            .unwrap();
    }
    assert!(!db.archive_conversation(&prepared).unwrap().replayed);
    assert!(metadata(&db.raw.lock().unwrap(), &command).0);
}

#[tokio::test]
async fn commit_unknown_before_commit_stays_unconfirmed_until_reopen() {
    let (dir, db, command, prepared) = fixture().await;
    let error = db
        .archive_conversation_owned(&prepared, Fault::BeforeCommit)
        .unwrap_err();
    assert_eq!(error.code(), "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        error.diagnostic().unwrap().operation_id,
        command.operation_id
    );
    assert!(matches!(
        db.archive_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.archive_conversation(&prepared).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("archive.sqlite3")).unwrap();
    assert!(!metadata(&observer, &command).0);
    assert_eq!(receipt_count(&observer), 0);
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(matches!(
        reopened.archive_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(
        !reopened
            .archive_conversation(
                &SqliteConversationCatalog::new(reopened.clone())
                    .validate_archive(command.clone())
                    .await
                    .unwrap()
            )
            .unwrap()
            .replayed
    );
    assert!(metadata(&reopened.raw.lock().unwrap(), &command).0);
}

#[tokio::test]
async fn committed_but_lost_reply_is_observable_without_unblocking_or_reapplying() {
    let (dir, db, command, prepared) = fixture().await;
    assert!(matches!(
        db.archive_conversation_owned(&prepared, Fault::AfterCommit),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    let proof = match db.archive_status(&command).unwrap() {
        SqlImmediateStatus::Committed(value) => value,
        _ => panic!("committed proof must be readable"),
    };
    assert_eq!(proof.operation_id, command.operation_id);
    assert_eq!(proof.sql_fence.position, 0);
    assert!(db.archive_conversation(&prepared).is_err());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("archive.sqlite3")).unwrap();
    assert!(metadata(&observer, &command).0);
    assert_eq!(receipt_count(&observer), 1);
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert_eq!(
        reopened
            .archive_conversation(
                &SqliteConversationCatalog::new(reopened.clone())
                    .validate_archive(command.clone())
                    .await
                    .unwrap()
            )
            .unwrap(),
        proof
    );
    assert_eq!(receipt_count(&reopened.raw.lock().unwrap()), 1);
}

#[tokio::test]
async fn same_connection_uncommitted_archived_and_receipt_are_not_a_committed_status() {
    let (dir, db, command, prepared) = fixture().await;
    assert!(matches!(
        db.archive_conversation_owned(&prepared, Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    {
        let sql = db.raw.lock().unwrap();
        assert!(!sql.is_autocommit());
        assert!(metadata(&sql, &command).0);
        assert_eq!(receipt_count(&sql), 1);
    }
    let observer = Connection::open(dir.path().join("archive.sqlite3")).unwrap();
    assert!(!metadata(&observer, &command).0);
    assert_eq!(receipt_count(&observer), 0);
    for _ in 0..2 {
        assert!(matches!(
            db.archive_status(&command).unwrap(),
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
        db.archive_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.lock().is_err());
    drop(observer);
    drop(db);
    let reopened = open(dir.path());
    assert!(
        !reopened
            .archive_conversation(
                &SqliteConversationCatalog::new(reopened.clone())
                    .validate_archive(command.clone())
                    .await
                    .unwrap()
            )
            .unwrap()
            .replayed
    );
}

#[tokio::test]
async fn a_real_deferred_constraint_commit_failure_rolls_back_both_rows_and_blocks() {
    let (dir, db, command, prepared) = fixture().await;
    db.lock().unwrap().execute_batch("create table archive_parent(id text primary key);
      create table archive_child(id text primary key,parent_id text references archive_parent(id) deferrable initially deferred);
      create trigger reject_archive_commit after update of archived on agent_conversations begin insert into archive_child values(new.id,'missing'); end;").unwrap();
    let error = db.archive_conversation(&prepared).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert!(db.raw.lock().unwrap().is_autocommit());
    assert!(db.lock().is_err());
    let observer = Connection::open(dir.path().join("archive.sqlite3")).unwrap();
    assert!(!metadata(&observer, &command).0);
    assert_eq!(receipt_count(&observer), 0);
    assert!(matches!(
        db.archive_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
}

#[tokio::test]
async fn corrupt_own_result_blocks_a_write_but_observation_does_not_repair_it() {
    let (_dir, db, command, prepared) = fixture().await;
    db.archive_conversation(&prepared).unwrap();
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set result_json='[]'",
            [],
        )
        .unwrap();
    assert!(matches!(
        db.archive_status(&command),
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
        db.archive_conversation(&prepared),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(db.lock().is_err());
    assert!(matches!(
        db.archive_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(metadata(&db.raw.lock().unwrap(), &command).0);
}

#[tokio::test]
async fn closed_owner_keeps_original_proof_and_does_not_admit_a_new_operation() {
    let (_dir, db, command, prepared) = fixture().await;
    let proof = db.archive_conversation(&prepared).unwrap();
    db.shutdown().unwrap();
    assert!(
        matches!(db.archive_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    let unknown = ArchiveConversation {
        operation_id: Uuid::new_v4(),
        ..command.clone()
    };
    assert!(matches!(
        db.archive_status(&unknown).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { closed: true, .. }
    ));
    assert!(matches!(
        db.archive_replay(&unknown),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
}

#[tokio::test]
async fn status_rejects_wrong_epoch_or_future_receipt_without_mutating_the_owner() {
    for corrupt in ["epoch", "future"] {
        let (_dir, db, command, prepared) = fixture().await;
        db.archive_conversation(&prepared).unwrap();
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
            db.archive_status(&command),
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
