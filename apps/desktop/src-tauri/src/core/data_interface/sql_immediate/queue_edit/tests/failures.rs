use super::*;

#[test]
fn before_commit_rollback_and_lost_ack_keep_the_original_r_until_paired_reopen() {
    for (fault, exists) in [
        (Fault::BeforeCommit, false),
        (Fault::Rollback, false),
        (Fault::AfterCommit, true),
    ] {
        let f = Fixture::new();
        assert!(matches!(
            f.db.edit_input_queue_owned(&f.intent, fault),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Unknown(_)))
        ));
        assert_eq!(f.count(), if exists { (1, 1) } else { (0, 0) });
        assert_eq!(
            matches!(
                f.db.input_queue_edit_status(&f.intent).unwrap(),
                QueueEditStatus::Committed { .. }
            ),
            exists
        );
        assert!(matches!(
            f.db.edit_input_queue(&f.intent),
            Err(QueueEditFailure::Storage(
                SqlImmediateFailure::RecoveryRequired(_)
            ))
        ));
        let f = f.reopen();
        let (queue, proof) = committed(f.db.edit_input_queue(&f.intent).unwrap());
        assert_eq!(queue.revision(), 1);
        assert_eq!(proof.replayed, exists);
        assert_eq!(f.count(), (1, 1));
    }
}

#[test]
fn deferred_commit_failure_is_unknown_with_sqlite_cause_and_never_a_domain_refusal() {
    let f = Fixture::new();
    f.db.raw.lock().unwrap().execute_batch("create table queue_parent(id text primary key); create table queue_child(id text primary key,parent_id text references queue_parent(id) deferrable initially deferred); create trigger fail_queue_commit after insert on data_sql_operation_receipts when new.kind='agent.input_queue.edit' begin insert into queue_child values(new.operation_id,'missing'); end;").unwrap();
    let QueueEditFailure::Storage(error) = f.db.edit_input_queue(&f.intent).unwrap_err() else {
        panic!("real SQL failure required")
    };
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert_eq!(f.count(), (0, 0));
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(),QueueEditStatus::Unconfirmed(detail) if detail.safe_cause=="owner_blocked_receipt_unconfirmed")
    );
    let f = f.reopen();
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_queue_commit;")
        .unwrap();
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent).unwrap(),
        QueueEditStatus::NotFound
    ));
    assert_eq!(
        committed(f.db.edit_input_queue(&f.intent).unwrap())
            .0
            .revision(),
        1
    );
}

#[test]
fn failed_proof_insert_rolls_back_input_identity_and_preserves_safe_sql_codes() {
    let f = Fixture::new();
    f.db.raw.lock().unwrap().execute_batch("create trigger reject_queue_proof before insert on data_sql_operation_receipts when new.kind='agent.input_queue.edit' begin select raise(abort,'private synthetic input text'); end;").unwrap();
    let QueueEditFailure::Storage(error) = f.db.edit_input_queue(&f.intent).unwrap_err() else {
        panic!("real SQL failure required")
    };
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private synthetic")
    );
    assert_eq!(f.count(), (0, 0));
    assert_eq!(
        f.observer()
            .query_row(
                "select count(*) from agent_input_queue_identities",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent).unwrap(),
        QueueEditStatus::NotFound
    ));
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("drop trigger reject_queue_proof;")
        .unwrap();
    f.db.edit_input_queue(&f.intent).unwrap();
}

#[test]
fn actual_rollback_failure_overrides_both_business_rejection_and_legacy_success() {
    for legacy in [false, true] {
        let f = Fixture::new();
        let mut intent = f.intent.clone();
        if legacy {
            f.legacy();
        } else if let QueueEdit::Add(input) = &mut intent.edit {
            input.text.clear();
            input.user_input.clear();
        }
        let QueueEditFailure::Storage(error) =
            f.db.edit_input_queue_owned(&intent, Fault::Rollback)
                .unwrap_err()
        else {
            panic!("rollback Unknown has priority")
        };
        assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
        let detail = error.diagnostic().unwrap();
        assert_eq!(detail.phase, "rollback");
        assert_eq!(detail.safe_cause, "sql_rollback_failed");
        assert!(detail.sqlite_extended_code.is_some());
        assert_eq!(f.count(), if legacy { (1, 0) } else { (0, 0) });
        assert!(f.db.lock().is_err());
    }
}

#[test]
fn matching_proof_with_missing_result_is_corrupt_but_read_error_keeps_sqlite_codes() {
    let f = Fixture::new();
    f.db.edit_input_queue(&f.intent).unwrap();
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("alter table agent_input_queue_receipts rename to hidden_queue_receipts;")
        .unwrap();
    for result in [
        f.db.edit_input_queue(&f.intent).map(|_| ()),
        f.db.input_queue_edit_status(&f.intent).map(|_| ()),
    ] {
        let QueueEditFailure::Storage(error) = result.unwrap_err() else {
            panic!("SQL diagnostic required")
        };
        assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
        assert_eq!(
            error.diagnostic().unwrap().sqlite_extended_code,
            Some(rusqlite::ffi::SQLITE_ERROR)
        );
    }
    assert!(f.db.lock().is_ok());
    f.db.raw.lock().unwrap().execute_batch("alter table hidden_queue_receipts rename to agent_input_queue_receipts; delete from agent_input_queue_receipts;").unwrap();
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Corrupt(_)))
    ));
    assert!(f.db.lock().is_ok());
    assert!(matches!(
        f.db.edit_input_queue(&f.intent),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Corrupt(_)))
    ));
    assert!(f.db.lock().is_err());
}

#[test]
fn altered_frozen_result_is_not_adopted_and_write_blocks_the_corrupt_owner() {
    let f = Fixture::new();
    f.db.edit_input_queue(&f.intent).unwrap();
    f.db.raw.lock().unwrap().execute("update agent_input_queue_receipts set result_json=json_set(result_json,'$.inputs[0][1]','altered synthetic text') where request_id=?1",[f.intent.request_id.to_string()]).unwrap();
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Corrupt(_)))
    ));
    assert!(f.db.lock().is_ok());
    assert!(matches!(
        f.db.edit_input_queue(&f.intent),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Corrupt(_)))
    ));
    assert!(f.db.lock().is_err());
}
