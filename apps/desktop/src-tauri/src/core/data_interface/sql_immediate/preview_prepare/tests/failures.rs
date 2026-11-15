use super::*;

#[test]
fn before_after_commit_and_open_transaction_unknown_recover_only_the_original_intent() {
    for (fault, committed) in [
        (Fault::BeforeCommit, false),
        (Fault::AfterCommit, true),
        (Fault::Rollback, false),
    ] {
        let f = Fixture::new();
        assert!(matches!(
            f.db.prepare_queue_preview_owned(&f.intent, f.source(), fault),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        let status = f.db.read_queue_preview_prepare_status(&f.intent).unwrap();
        assert_eq!(
            matches!(status, QueuePreviewPrepareStatus::Committed { .. }),
            committed
        );
        if !committed {
            assert!(matches!(status, QueuePreviewPrepareStatus::Unconfirmed(_)));
        }
        assert!(f.db.replay_queue_preview_prepare(&f.intent).is_err());
        assert!(f.db.prepare_queue_preview(&f.intent, f.source()).is_err());
        let observer = f.observer();
        assert_eq!(
            f.counts(&observer),
            if committed { (1, 1, 1) } else { (0, 0, 0) }
        );
        assert_eq!(
            observer
                .query_row(
                    "select revision from agent_input_queues where conversation_id=?1",
                    [f.intent.conversation_id.to_string()],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            if committed { 2 } else { 1 }
        );
        drop(observer);
        let f = f.reopen();
        assert_eq!(
            matches!(
                f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),
                QueuePreviewPrepareStatus::Committed { .. }
            ),
            committed
        );
        assert_eq!(
            f.db.replay_queue_preview_prepare(&f.intent)
                .unwrap()
                .is_some(),
            committed
        );
        assert_eq!(f.prepare().queue.revision(), 2);
        assert_eq!(f.counts(&f.observer()), (1, 1, 1));
    }
}

#[test]
fn actual_deferred_commit_failure_rolls_back_claim_grants_and_both_receipts() {
    let f = Fixture::new();
    f.db.lock().unwrap().execute_batch("create table prepare_commit_parent(id text primary key); create table prepare_commit_child(id text primary key,parent_id text references prepare_commit_parent(id) deferrable initially deferred); create trigger fail_prepare_commit after insert on data_sql_operation_receipts when new.kind='agent.queue.preview.prepare' begin insert into prepare_commit_child(id,parent_id) values(new.operation_id,'missing'); end;").unwrap();
    let error =
        f.db.prepare_queue_preview(&f.intent, f.source())
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Unconfirmed(detail) if detail.safe_cause=="owner_blocked_receipt_unconfirmed")
    );
    let observer = f.observer();
    assert_eq!(f.counts(&observer), (0, 0, 0));
    assert_eq!(
        observer
            .query_row(
                "select count(*) from agent_turn_resource_grants",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row(
                "select count(*) from agent_input_queue_previews",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert!(f.db.raw.lock().unwrap().is_autocommit());
    drop(observer);
    let f = f.reopen();
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_prepare_commit;")
        .unwrap();
    assert!(matches!(
        f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),
        QueuePreviewPrepareStatus::NotFound
    ));
    assert_eq!(f.prepare().queue.revision(), 2);
}

#[test]
fn proof_insert_failure_rolls_back_all_materialization_writes_with_safe_driver_code() {
    let f = Fixture::new();
    f.db.lock().unwrap().execute_batch("create trigger reject_prepare_proof before insert on data_sql_operation_receipts when new.kind='agent.queue.preview.prepare' begin select raise(abort,'private synthetic prepare'); end;").unwrap();
    let before = f.queue();
    let error =
        f.db.prepare_queue_preview(&f.intent, f.source())
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(1811));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private synthetic")
    );
    assert_eq!(f.queue(), before);
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    let sql = f.db.lock().unwrap();
    for table in [
        "agent_queue_submission_origins",
        "agent_input_queue_previews",
        "agent_turn_resource_grants",
    ] {
        assert_eq!(
            sql.query_row(&format!("select count(*) from {table}"), [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    sql.execute_batch("drop trigger reject_prepare_proof;")
        .unwrap();
    drop(sql);
    assert_eq!(f.prepare().queue.revision(), 2);
}

#[test]
fn open_transaction_status_never_reads_its_uncommitted_proof_or_rolls_back() {
    let f = Fixture::new();
    assert!(matches!(
        f.db.prepare_queue_preview_owned(&f.intent, f.source(), Fault::Rollback),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    for _ in 0..2 {
        assert!(
            matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Unconfirmed(detail) if detail.safe_cause=="sql_transaction_open")
        );
        assert!(!f.db.raw.lock().unwrap().is_autocommit());
    }
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    let f = f.reopen();
    assert!(matches!(
        f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),
        QueuePreviewPrepareStatus::NotFound
    ));
}

#[test]
fn proof_without_matching_frozen_business_result_is_not_adopted_or_repaired() {
    for missing in [false, true] {
        let f = Fixture::new();
        f.prepare();
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
            f.db.raw.lock().unwrap().execute("update agent_input_queue_receipts set result_json=json_set(result_json,'$.preparing',null) where request_id=?1",[f.intent.request_id.to_string()]).unwrap();
        }
        assert!(matches!(
            f.db.read_queue_preview_prepare_status(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert!(f.db.lock().is_ok());
        assert!(matches!(
            f.db.replay_queue_preview_prepare(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert!(f.db.lock().is_err());
    }
}
