use super::*;

fn observer(f: &Fixture) -> Connection {
    Connection::open(f.root.path().join("queue.sqlite3")).unwrap()
}

fn receipt_count(db: &Connection, request: Uuid) -> (u64, u64) {
    let business = db
        .query_row(
            "select count(*) from agent_input_queue_receipts where request_id=?1",
            [request.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let sql = db
        .query_row(
            "select count(*) from data_sql_operation_receipts where operation_id=?1",
            [request.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    (business, sql)
}

fn preparation_count(db: &Connection, request: Uuid) -> (u64, u64, u64) {
    let submission = db
        .query_row(
            "select count(*) from agent_submissions where request_id=?1",
            [request.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let header = db
        .query_row(
            "select count(*) from agent_compaction_preparations where request_id=?1",
            [request.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let pointer = db
        .query_row(
            "select count(*) from agent_input_queue_previews where request_id=?1",
            [request.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    (submission, header, pointer)
}

#[test]
fn rejected_receipt_rolls_back_submission_header_claim_and_both_receipts() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let before = f.q();
    f.db.lock().unwrap().execute_batch("create trigger reject_compact_sql_receipt before insert on data_sql_operation_receipts when new.kind='agent.queue.compaction.prepare' begin select raise(abort,'private fixture text'); end;").unwrap();
    let error =
        f.db.prepare_queued_compaction(&command, f.source(&command))
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unavailable(_)));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private fixture text")
    );
    assert_eq!(f.q(), before);
    let sql = observer(&f);
    assert_eq!(receipt_count(&sql, command.request_id), (0, 0));
    assert_eq!(preparation_count(&sql, command.request_id), (0, 0, 0));
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger reject_compact_sql_receipt;")
        .unwrap();
    f.prepare(&command);
}

#[test]
fn unknown_prepare_never_becomes_absent_or_write_permission_before_reopen() {
    for (fault, committed) in [
        (Fault::BeforeCommit, false),
        (Fault::Rollback, false),
        (Fault::AfterCommit, true),
    ] {
        let f = Fixture::new();
        let command = f.intent(Uuid::new_v4(), None);
        let error =
            f.db.prepare_queued_compaction_owned(&command, f.source(&command), fault)
                .unwrap_err();
        assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
        assert!(matches!(
            f.db.replay_queued_compaction_prepare(&command),
            Err(SqlImmediateFailure::RecoveryRequired(_))
        ));
        assert!(matches!(
            f.db.read_queued_compaction(&ReadIntent {
                conversation_id: f.conversation,
                manuscript_id: f.manuscript,
                input_id: f.input,
                request_id: Some(command.request_id)
            }),
            Err(SqlImmediateFailure::RecoveryRequired(_))
        ));
        let sql = observer(&f);
        let expected = u64::from(committed);
        assert_eq!(
            receipt_count(&sql, command.request_id),
            (expected, expected)
        );
        assert_eq!(
            preparation_count(&sql, command.request_id),
            (expected, expected, expected)
        );
        drop(sql);
        let f = f.reopen();
        let replay = f.db.replay_queued_compaction_prepare(&command).unwrap();
        assert_eq!(replay.is_some(), committed);
        let state = f.prepare(&command);
        assert_eq!(state.queue.preparing(), Some(f.input));
        assert_eq!(receipt_count(&observer(&f), command.request_id), (1, 1));
    }
}

#[test]
fn actual_deferred_commit_failure_rolls_back_every_row_and_blocks_owner() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    f.db.lock().unwrap().execute_batch("create table compact_commit_parent(id text primary key); create table compact_commit_child(id text primary key,parent_id text references compact_commit_parent(id) deferrable initially deferred); create trigger fail_compact_commit after insert on agent_compaction_preparations begin insert into compact_commit_child(id,parent_id) values(new.request_id,'missing'); end;").unwrap();
    let error =
        f.db.prepare_queued_compaction(&command, f.source(&command))
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    assert_eq!(error.diagnostic().unwrap().phase, "commit");
    assert_eq!(error.diagnostic().unwrap().sqlite_extended_code, Some(787));
    let sql = observer(&f);
    assert_eq!(receipt_count(&sql, command.request_id), (0, 0));
    assert_eq!(preparation_count(&sql, command.request_id), (0, 0, 0));
    drop(sql);
    assert!(f.db.raw.lock().unwrap().is_autocommit());
    assert!(f.db.replay_queued_compaction_prepare(&command).is_err());
}

#[test]
fn settlement_rollback_preserves_current_claim_and_lost_ack_replays_after_reopen() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&command);
    f.terminal(&state, TurnOutcome::Completed);
    let settle = f.settle(&command);
    let before = f.q();
    f.db.lock().unwrap().execute_batch("create trigger reject_compact_settle before insert on data_sql_operation_receipts when new.kind='agent.queue.compaction.settle' begin select raise(abort,'fixture rejection'); end;").unwrap();
    assert!(matches!(
        f.db.settle_queued_compaction(&settle),
        Err(SqlImmediateFailure::Unavailable(_))
    ));
    assert_eq!(f.q(), before);
    assert_eq!(receipt_count(&observer(&f), settle.operation_id), (0, 0));
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger reject_compact_settle;")
        .unwrap();
    assert!(matches!(
        f.db.settle_queued_compaction_owned(&settle, Fault::AfterCommit),
        Err(SqlImmediateFailure::Unknown(_))
    ));
    assert!(f.db.settle_queued_compaction(&settle).is_err());
    assert_eq!(receipt_count(&observer(&f), settle.operation_id), (1, 1));
    let f = f.reopen();
    let replay = f.db.settle_queued_compaction(&settle).unwrap();
    assert!(replay.queue.inputs().is_empty());
    assert_eq!(replay.queue.revision(), before.revision() + 1);
    assert_eq!(f.q(), replay.queue);
    let original = f.prepare(&command);
    assert_eq!(original.queue, state.queue);
    assert!(original.current);
    assert_eq!(original.outcome, None);
}

#[test]
fn a_changed_current_payload_is_never_consumed_using_an_old_terminal_fact() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&command);
    f.terminal(&state, TurnOutcome::Completed);
    f.db.lock().unwrap().execute("update agent_input_queues set snapshot_json=json_set(snapshot_json,'$.inputs[0][1]','newer payload') where conversation_id=?1",[f.conversation.to_string()]).unwrap();
    let before = f.q();
    let settle = f.settle(&command);
    assert!(matches!(
        f.db.settle_queued_compaction(&settle),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(f.q(), before);
    assert_eq!(before.inputs()[0].text, "newer payload");
    assert_eq!(receipt_count(&observer(&f), settle.operation_id), (0, 0));
}

#[test]
fn cross_family_and_missing_sql_proof_cannot_adopt_existing_business_rows() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    f.prepare(&command);
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set kind='foreign.family' where operation_id=?1",
            [command.request_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.replay_queued_compaction_prepare(&command),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [command.request_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.replay_queued_compaction_prepare(&command),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    assert!(f.db.lock().is_err());
}
