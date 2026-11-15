use super::*;

#[test]
fn foreign_command_family_is_conflict_before_decoding_its_non_unit_result() {
    let (_dir, db, command) = fixture();
    db.authorize_rollback(&command).unwrap();
    db.raw.lock().unwrap().execute(
        "update data_sql_operation_receipts set kind='other.family',result_json='{\"value\":7}' where operation_id=?1",
        [command.operation_id.to_string()],
    ).unwrap();
    assert!(matches!(
        db.rollback_authorization_status(&command),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.authorize_rollback(&command),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(db.lock().is_ok());
}

#[test]
fn corrupt_own_receipt_blocks_writes_but_status_never_repairs_or_clears_it() {
    let (_dir, db, command) = fixture();
    db.authorize_rollback(&command).unwrap();
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set result_json='[]' where operation_id=?1",
            [command.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        db.rollback_authorization_status(&command),
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
        db.authorize_rollback(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(db.lock().is_err());
    assert!(matches!(
        db.rollback_authorization_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .blocked
    );
    assert!(db.raw.lock().unwrap().is_autocommit());
}

#[test]
fn commit_unknown_before_commit_blocks_and_reopens_without_a_false_receipt() {
    let (dir, db, command) = fixture();
    let error = db
        .authorize_rollback_owned(&command, Fault::BeforeCommit)
        .unwrap_err();
    let metadata = error.diagnostic().unwrap();
    assert_eq!(error.code(), "DATA_SQL_COMMIT_UNKNOWN");
    assert!(metadata.epoch.is_some());
    assert_eq!(metadata.sql_fence.as_deref(), Some("0"));
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(matches!(
        db.authorize_rollback(&command),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
    assert!(db.lock().is_err());
    assert!(
        db.write(pending(&command, Uuid::new_v4()), WriteOptions::default())
            .is_err()
    );
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    assert_eq!(count(&observer, "grant"), 0);
    assert_eq!(count(&observer, "receipt"), 0);
    drop(db);
    let reopened = open(dir.path());
    assert!(matches!(
        reopened.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(!reopened.authorize_rollback(&command).unwrap().replayed);
}

#[test]
fn commit_ack_lost_after_commit_is_inspectable_but_does_not_unblock_writes() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.authorize_rollback_owned(&command, Fault::AfterCommit),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    assert!(
        matches!(db.rollback_authorization_status(&command).unwrap(),SqlImmediateStatus::Committed(receipt) if receipt.operation_id==command.operation_id)
    );
    assert!(db.authorize_rollback(&command).is_err());
    assert!(db.lock().is_err());
    assert!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .blocked
    );
    drop(db);
    let reopened = open(dir.path());
    let replay = reopened.authorize_rollback(&command).unwrap();
    assert!(replay.replayed);
    assert_eq!(count(&reopened.raw.lock().unwrap(), "receipt"), 1);
}

#[test]
fn rollback_failure_never_treats_same_connection_uncommitted_receipt_as_success() {
    let (dir, db, command) = fixture();
    assert!(matches!(
        db.authorize_rollback_owned(&command, Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    {
        let sql = db.raw.lock().unwrap();
        assert!(!sql.is_autocommit());
        assert_eq!(count(&sql, "receipt"), 1);
    }
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    assert_eq!(count(&observer, "receipt"), 0);
    for _ in 0..2 {
        assert!(matches!(
            db.rollback_authorization_status(&command).unwrap(),
            SqlImmediateStatus::Unconfirmed {
                blocked: true,
                reason: "sql_transaction_open",
                ..
            }
        ));
        assert!(!db.raw.lock().unwrap().is_autocommit());
    }
    // Only this test's explicit recovery cleanup rolls back, never status.
    db.raw.lock().unwrap().execute_batch("rollback;").unwrap();
    assert!(db.lock().is_err());
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    drop(db);
    let reopened = open(dir.path());
    assert!(!reopened.authorize_rollback(&command).unwrap().replayed);
}

#[test]
fn real_sql_commit_failure_rolls_back_both_rows_and_keeps_the_owner_blocked() {
    let (dir, db, command) = fixture();
    db.lock().unwrap().execute_batch(
        "create table sql_commit_parent(id text primary key);
         create table sql_commit_child(id text primary key,parent_id text references sql_commit_parent(id) deferrable initially deferred);
         create trigger sql_fail_deferred after insert on agent_rollback_authority begin
           insert into sql_commit_child(id,parent_id) values(new.request_id,'missing');
         end;"
    ).unwrap();
    let error = db.authorize_rollback(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    let detail = error.diagnostic().unwrap();
    assert_eq!(detail.phase, "commit");
    assert_eq!(detail.sqlite_extended_code, Some(787));
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    assert_eq!(count(&observer, "grant"), 0);
    assert_eq!(count(&observer, "receipt"), 0);
    assert!(db.raw.lock().unwrap().is_autocommit());
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.lock().is_err());
}

#[test]
fn rejected_sql_statement_has_confirmed_rollback_and_safe_actionable_codes() {
    let (_dir, db, command) = fixture();
    db.lock().unwrap().execute_batch("create trigger sql_reject_grant before insert on agent_rollback_authority begin select raise(abort,'sensitive fixture body'); end;").unwrap();
    let error = db.authorize_rollback(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("sensitive fixture")
    );
    {
        let sql = db.lock().unwrap();
        assert!(sql.is_autocommit());
        assert_eq!(count(&sql, "receipt"), 0);
        assert_eq!(count(&sql, "grant"), 0);
        sql.execute_batch("drop trigger sql_reject_grant;").unwrap();
    }
    db.authorize_rollback(&command).unwrap();
}

#[test]
fn closed_owner_remains_closed_while_sql_registration_status_is_readable() {
    let (_dir, db, command) = fixture();
    db.authorize_rollback(&command).unwrap();
    db.shutdown().unwrap();
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    let mut missing = command.clone();
    missing.operation_id = Uuid::new_v4();
    assert!(matches!(
        db.rollback_authorization_status(&missing).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { closed: true, .. }
    ));
    assert!(matches!(
        db.authorize_rollback(&missing),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
    assert!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .closed
    );
}

#[test]
fn sql_mutex_poison_is_recovery_required_without_creating_a_registration() {
    let (_dir, db, command) = fixture();
    let poison = Arc::clone(&db);
    assert!(
        std::thread::spawn(move || {
            let _guard = poison.raw.lock().unwrap();
            panic!("synthetic SQL mutex poison");
        })
        .join()
        .is_err()
    );
    assert!(matches!(
        db.authorize_rollback(&command),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
    assert!(matches!(
        db.rollback_authorization_status(&command),
        Err(SqlImmediateFailure::RecoveryRequired(_))
    ));
    assert!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .blocked
    );
}

#[test]
fn status_accepts_sql_ahead_of_cached_head_but_does_not_change_cached_heads() {
    let (_dir, db, command) = fixture();
    let receipt = db.authorize_rollback(&command).unwrap();
    db.write(pending(&command, Uuid::new_v4()), WriteOptions::default())
        .unwrap();
    // Model an acknowledged SQL prefix ahead of the last confirmed in-memory
    // materialization after a lost projector acknowledgement. No log is read.
    db.raw
        .lock()
        .unwrap()
        .execute("update data_runtime_state set materialized_position=1", [])
        .unwrap();
    let before = db
        .owner
        .as_ref()
        .unwrap()
        .inspection_guard()
        .unwrap()
        .snapshot();
    assert_eq!(before.accepted_position, 1);
    assert_eq!(before.materialized_position, 0);
    assert!(
        matches!(db.rollback_authorization_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.sql_fence==receipt.sql_fence)
    );
    let after = db
        .owner
        .as_ref()
        .unwrap()
        .inspection_guard()
        .unwrap()
        .snapshot();
    assert_eq!(before, after);
}

#[test]
fn status_rejects_sql_prefix_beyond_confirmed_accepted_without_changing_owner() {
    let (_dir, db, command) = fixture();
    db.authorize_rollback(&command).unwrap();
    db.raw
        .lock()
        .unwrap()
        .execute("update data_runtime_state set materialized_position=1", [])
        .unwrap();
    let before = db
        .owner
        .as_ref()
        .unwrap()
        .inspection_guard()
        .unwrap()
        .snapshot();
    assert_eq!(before.accepted_position, 0);
    assert!(matches!(
        db.rollback_authorization_status(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    let after = db
        .owner
        .as_ref()
        .unwrap()
        .inspection_guard()
        .unwrap()
        .snapshot();
    assert_eq!(before, after);
}

#[test]
fn receipt_insert_failure_rolls_back_the_prior_grant_and_allows_same_operation_retry() {
    let (_dir, db, command) = fixture();
    db.lock()
        .unwrap()
        .execute_batch(
            "create trigger sql_reject_receipt before insert on data_sql_operation_receipts
         when exists(select 1 from agent_rollback_authority where request_id=new.operation_id)
         begin select raise(abort,'receipt-stage-rejected'); end;",
        )
        .unwrap();
    let error = db.authorize_rollback(&command).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "receipt");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert!(
        !db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .blocked
    );
    {
        let sql = db.lock().unwrap();
        assert!(sql.is_autocommit());
        assert_eq!(count(&sql, "grant"), 0);
        assert_eq!(count(&sql, "receipt"), 0);
        sql.execute_batch("drop trigger sql_reject_receipt;")
            .unwrap();
    }
    let receipt = db.authorize_rollback(&command).unwrap();
    assert_eq!(receipt.operation_id, command.operation_id);
    assert!(!receipt.replayed);
    let sql = db.raw.lock().unwrap();
    assert_eq!(count(&sql, "grant"), 1);
    assert_eq!(count(&sql, "receipt"), 1);
}

#[test]
fn status_rejects_damaged_prefix_epoch_and_future_receipt_without_repairing_them() {
    for scenario in ["head_behind", "wrong_epoch", "future_receipt"] {
        // Each case has its own real pair; no damaged state is reused.
        let (_dir, db, command) = fixture();
        if scenario == "head_behind" {
            db.write(
                pending(&command, Uuid::new_v4()),
                WriteOptions {
                    mode: research_data_runtime::WriteMode::Immediate,
                    ..WriteOptions::default()
                },
            )
            .unwrap();
        }
        db.authorize_rollback(&command).unwrap();
        {
            let sql = db.raw.lock().unwrap();
            match scenario {
                "head_behind" => {
                    sql.execute("update data_runtime_state set materialized_position=0", [])
                        .unwrap();
                }
                "wrong_epoch" => {
                    sql.execute(
                        "update data_runtime_state set epoch=?1",
                        [Uuid::new_v4().to_string()],
                    )
                    .unwrap();
                }
                "future_receipt" => {
                    sql.execute(
                        "update data_sql_operation_receipts set sql_fence_position=1",
                        [],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let snapshot = || {
            db.owner
                .as_ref()
                .unwrap()
                .inspection_guard()
                .unwrap()
                .snapshot()
        };
        let sql_state = || {
            let sql = db.raw.lock().unwrap();
            sql.query_row(
                "select s.epoch,s.materialized_position,r.sql_fence_position
                 from data_runtime_state s cross join data_sql_operation_receipts r
                 where s.singleton=1 and r.operation_id=?1",
                [command.operation_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                },
            )
            .unwrap()
        };
        let before_owner = snapshot();
        let before_sql = sql_state();
        let error = db.rollback_authorization_status(&command).unwrap_err();
        assert!(
            matches!(error, SqlImmediateFailure::Corrupt(_)),
            "{scenario}"
        );
        assert_eq!(
            error.diagnostic().unwrap().safe_cause,
            if scenario == "future_receipt" {
                "sql_receipt_invalid"
            } else {
                "sql_pair_or_fence_mismatch"
            },
            "{scenario}"
        );
        assert_eq!(error.diagnostic().unwrap().phase, "status");
        assert_eq!(snapshot(), before_owner, "{scenario}");
        assert_eq!(sql_state(), before_sql, "{scenario}");
    }
}
