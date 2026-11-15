use super::retirement::cancellation;
use super::*;

#[test]
fn discovers_exact_retirement_and_reopens_the_same_cancellation_proof() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    assert!(f.prepare(&prepare).retirement.is_none());
    let intent = cancellation(&f, &prepare);
    let (_, proof) = f.db.cancel_queue_preview(&intent).unwrap();
    for request in [None, Some(prepare.request_id)] {
        let Some(QueuedCompactionRead::Compact(state)) = f.read(request) else {
            panic!("original compact missing")
        };
        assert!(!state.current);
        assert!(state.outcome.is_none());
        let retired = state.retirement.unwrap();
        assert_eq!(retired.intent.request_id, intent.request_id);
        assert_eq!(retired.intent.preview_request_id, prepare.request_id);
        assert_eq!(retired.receipt.fingerprint, proof.fingerprint);
        assert_eq!(retired.receipt.epoch, proof.epoch);
        assert_eq!(retired.queue, f.q());
        assert_ne!(state.queue, retired.queue);
        let wire = serde_json::to_value(
            crate::agent::writing_queue_preview::CancelStatusResponse::committed(*retired).unwrap(),
        )
        .unwrap();
        assert_eq!(wire["schemaVersion"], 1);
        assert_eq!(wire["intent"]["requestId"], intent.request_id.to_string());
        assert_eq!(
            wire["intent"]["previewRequestId"],
            prepare.request_id.to_string()
        );
        assert_eq!(wire["status"]["type"], "committed");
        assert_eq!(
            wire["status"]["proof"]["family"],
            "agent.queue.preview.cancel"
        );
        assert_eq!(wire["status"]["proof"]["epoch"], proof.epoch.to_string());
        assert_eq!(
            wire["status"]["proof"]["sqlFence"],
            proof.sql_fence.position.to_string()
        );
        assert_eq!(wire["status"]["proof"]["result"], serde_json::Value::Null);
    }
    let read = ReadIntent {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        input_id: f.input,
        request_id: Some(prepare.request_id),
    };
    let Fixture {
        root, db, store, ..
    } = f;
    drop(store);
    drop(db);
    let reopened = Database::open(
        Connection::open(root.path().join("queue.sqlite3")).unwrap(),
        &root.path().join("data-log"),
    )
    .unwrap();
    let Some(QueuedCompactionRead::Compact(state)) =
        reopened.read_queued_compaction(&read).unwrap()
    else {
        panic!("cold compact missing")
    };
    assert_eq!(
        state.retirement.unwrap().intent.request_id,
        intent.request_id
    );
}

#[test]
fn explicit_new_attempt_keeps_old_retirement_proof_and_new_claim_independent() {
    let f = Fixture::new();
    let original = f.intent(Uuid::new_v4(), None);
    f.prepare(&original);
    let intent = cancellation(&f, &original);
    let (released, _) = f.db.cancel_queue_preview(&intent).unwrap();
    f.edit(QueueEdit::ResumeInput(f.input));
    let next = f.intent(Uuid::new_v4(), Some(original.request_id));
    let prepared = f.prepare(&next);
    assert!(prepared.retirement.is_none());
    assert!(prepared.current);
    let latest = f.q();
    let Some(QueuedCompactionRead::Compact(old)) = f.read(Some(original.request_id)) else {
        panic!("old compact missing")
    };
    assert_eq!(old.retirement.unwrap().intent.request_id, intent.request_id);
    assert!(!old.current);
    assert_eq!(f.db.cancel_queue_preview(&intent).unwrap().0, released);
    assert_eq!(f.q(), latest);
    let Some(QueuedCompactionRead::Compact(current)) = f.read(None) else {
        panic!("new compact missing")
    };
    assert_eq!(current.record.request_id, next.request_id);
    assert!(current.retirement.is_none());
}

#[test]
fn business_cancel_without_typed_proof_is_neither_retirement_nor_new_attempt_permission() {
    let f = Fixture::new();
    let original = f.intent(Uuid::new_v4(), None);
    f.prepare(&original);
    let intent = cancellation(&f, &original);
    f.db.cancel_queue_preview(&intent).unwrap();
    f.db.lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [intent.request_id.to_string()],
        )
        .unwrap();
    let read = ReadIntent {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        input_id: f.input,
        request_id: Some(original.request_id),
    };
    assert!(f.db.read_queued_compaction(&read).is_err());
    f.edit(QueueEdit::ResumeInput(f.input));
    let next = f.intent(Uuid::new_v4(), Some(original.request_id));
    let before = f.q();
    assert!(
        f.db.prepare_queued_compaction(&next, f.source(&next))
            .is_err()
    );
    let sql = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
    let json: String = sql
        .query_row(
            "select snapshot_json from agent_input_queues where conversation_id=?1",
            [f.conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        input_queue::decode_prepared_snapshot(&json, f.conversation).unwrap(),
        before
    );
    assert!(
        crate::agent::repository::submissions::read(&sql, f.conversation, next.request_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn changed_cancel_epoch_family_version_fingerprint_or_fence_never_authorizes_read_or_restart() {
    use rusqlite::types::Value;
    for (sql, value) in [
        (
            "update data_sql_operation_receipts set epoch=?2 where operation_id=?1",
            Value::Text(Uuid::new_v4().to_string()),
        ),
        (
            "update data_sql_operation_receipts set kind=?2 where operation_id=?1",
            Value::Text("foreign.family".into()),
        ),
        (
            "update data_sql_operation_receipts set version=?2 where operation_id=?1",
            Value::Integer(2),
        ),
        (
            "update data_sql_operation_receipts set fingerprint=?2 where operation_id=?1",
            Value::Blob(vec![0; 32]),
        ),
        (
            "update data_sql_operation_receipts set sql_fence_position=?2 where operation_id=?1",
            Value::Integer(i64::MAX),
        ),
    ] {
        let f = Fixture::new();
        let original = f.intent(Uuid::new_v4(), None);
        f.prepare(&original);
        let intent = cancellation(&f, &original);
        f.db.cancel_queue_preview(&intent).unwrap();
        f.edit(QueueEdit::ResumeInput(f.input));
        let next = f.intent(Uuid::new_v4(), Some(original.request_id));
        let before = f.q();
        f.db.lock()
            .unwrap()
            .execute(sql, params![intent.request_id.to_string(), value])
            .unwrap();
        let read = ReadIntent {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            input_id: f.input,
            request_id: Some(original.request_id),
        };
        assert!(f.db.read_queued_compaction(&read).is_err());
        assert!(
            f.db.prepare_queued_compaction(&next, f.source(&next))
                .is_err()
        );
        let observer = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
        let json: String = observer
            .query_row(
                "select snapshot_json from agent_input_queues where conversation_id=?1",
                [f.conversation.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            input_queue::decode_prepared_snapshot(&json, f.conversation).unwrap(),
            before
        );
    }
}
