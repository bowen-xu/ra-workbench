use super::*;

#[test]
fn before_commit_rollback_and_lost_ack_stay_unknown_until_original_intent_is_reopened() {
    for (fault, committed) in [
        (Fault::BeforeCommit, false),
        (Fault::Rollback, false),
        (Fault::AfterCommit, true),
    ] {
        let f = Fixture::new();
        assert!(matches!(
            f.db.cancel_queue_preview_owned(&f.intent, fault),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        let status = f.db.queue_preview_cancel_status(&f.intent).unwrap();
        if committed {
            assert!(
                matches!(status,QueuePreviewCancelStatus::Committed{queue,..} if queue.revision()==3)
            );
        } else {
            assert!(matches!(status, QueuePreviewCancelStatus::Unconfirmed(_)));
        }
        assert!(f.db.cancel_queue_preview(&f.intent).is_err());
        let observer = f.observer();
        assert_eq!(f.count(&observer), if committed { (1, 1) } else { (0, 0) });
        assert_eq!(
            observer
                .query_row(
                    "select revision from agent_input_queues where conversation_id=?1",
                    [f.intent.conversation_id.to_string()],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            if committed { 3 } else { 2 }
        );
        drop(observer);
        let f = f.reopen();
        let status = f.db.queue_preview_cancel_status(&f.intent).unwrap();
        assert_eq!(
            matches!(status, QueuePreviewCancelStatus::Committed { .. }),
            committed
        );
        let (queue, proof) = f.db.cancel_queue_preview(&f.intent).unwrap();
        assert_eq!(proof.replayed, committed);
        assert_eq!(queue.revision(), 3);
        assert_eq!(f.count(&f.observer()), (1, 1));
    }
}

#[test]
fn a_real_deferred_commit_failure_keeps_original_queue_and_reopens_the_same_cancel() {
    let f = Fixture::new();
    f.db.lock().unwrap().execute_batch("create table cancel_commit_parent(id text primary key); create table cancel_commit_child(id text primary key,parent_id text references cancel_commit_parent(id) deferrable initially deferred); create trigger fail_cancel_commit after insert on data_sql_operation_receipts when new.kind='agent.queue.preview.cancel' begin insert into cancel_commit_child(id,parent_id) values(new.operation_id,'missing'); end;").unwrap();
    let error = f.db.cancel_queue_preview(&f.intent).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert!(
        matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Unconfirmed(detail) if detail.safe_cause=="owner_blocked_receipt_unconfirmed")
    );
    assert_eq!(f.count(&f.observer()), (0, 0));
    assert!(f.db.raw.lock().unwrap().is_autocommit());
    let f = f.reopen();
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_cancel_commit;")
        .unwrap();
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    assert_eq!(
        f.db.cancel_queue_preview(&f.intent).unwrap().0.revision(),
        3
    );
}

#[test]
fn sql_proof_rejection_rolls_back_business_cancel_and_retains_safe_driver_codes() {
    let f = Fixture::new();
    let before = f.queue();
    f.db.lock().unwrap().execute_batch("create trigger reject_cancel_proof before insert on data_sql_operation_receipts when new.kind='agent.queue.preview.cancel' begin select raise(abort,'private synthetic cancellation'); end;").unwrap();
    let error = f.db.cancel_queue_preview(&f.intent).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private synthetic")
    );
    assert_eq!(f.queue(), before);
    assert_eq!(f.count(&f.observer()), (0, 0));
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger reject_cancel_proof;")
        .unwrap();
    f.db.cancel_queue_preview(&f.intent).unwrap();
}

#[test]
fn status_never_reads_an_open_uncommitted_receipt_or_rolls_back_its_transaction() {
    let f = Fixture::new();
    assert!(matches!(
        f.db.cancel_queue_preview_owned(&f.intent, Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    assert!(!f.db.raw.lock().unwrap().is_autocommit());
    for _ in 0..2 {
        assert!(
            matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Unconfirmed(detail) if detail.safe_cause=="sql_transaction_open")
        );
        assert!(!f.db.raw.lock().unwrap().is_autocommit());
    }
    assert_eq!(f.count(&f.observer()), (0, 0));
    let f = f.reopen();
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
}

#[test]
fn matching_sql_proof_without_valid_business_result_is_never_adopted() {
    for missing in [false, true] {
        let f = Fixture::new();
        f.db.cancel_queue_preview(&f.intent).unwrap();
        if missing {
            f.db.raw
                .lock()
                .unwrap()
                .execute(
                    "delete from agent_input_queue_receipts where request_id=?1",
                    [f.intent.request_id.to_string()],
                )
                .unwrap();
        } else {
            f.db.raw.lock().unwrap().execute("update agent_input_queue_receipts set result_json=json_set(result_json,'$.paused',json('false')) where request_id=?1",[f.intent.request_id.to_string()]).unwrap();
        }
        assert!(matches!(
            f.db.queue_preview_cancel_status(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert!(f.db.lock().is_ok());
        assert!(matches!(
            f.db.cancel_queue_preview(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert!(f.db.lock().is_err());
    }
}
