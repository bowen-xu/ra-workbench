use super::*;
use crate::data_interface::{CancelPreviewIntent, QueuePreviewCancelStatus};

pub(super) fn cancellation(f: &Fixture, prepare: &PrepareIntent) -> CancelPreviewIntent {
    CancelPreviewIntent {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: Uuid::new_v4(),
        expected_queue_revision: f.q().revision(),
        input_id: f.input,
        preview_request_id: prepare.request_id,
    }
}

#[test]
fn retires_unstarted_compact_with_original_cancel_proof_and_replays_after_reopen() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let original = f.q();
    let (released, receipt) = f.db.cancel_queue_preview(&intent).unwrap();
    assert_eq!(released.inputs(), original.inputs());
    assert!(released.paused());
    assert_eq!(released.preparing(), None);
    assert_eq!(released.revision(), original.revision() + 1);
    assert!(
        input_queue::queued_compaction::require_startable_in(
            &f.db.lock().unwrap(),
            f.conversation,
            prepare.request_id,
        )
        .is_err()
    );
    assert_eq!(receipt.operation_id, intent.request_id);
    let kind: String =
        f.db.lock()
            .unwrap()
            .query_row(
                "select kind from data_sql_operation_receipts where operation_id=?1",
                [intent.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(kind, "agent.queue.preview.cancel");
    let (replayed, proof) = f.db.cancel_queue_preview(&intent).unwrap();
    assert_eq!(replayed, released);
    assert!(proof.replayed);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::Committed { .. }
    ));
    let Fixture {
        root, db, store, ..
    } = f;
    drop(store);
    drop(db);
    let sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
    let reopened = Database::open(sql, &root.path().join("data-log")).unwrap();
    assert!(matches!(
        reopened.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::Committed { .. }
    ));
    assert_eq!(reopened.cancel_queue_preview(&intent).unwrap().0, released);
    reopened
        .lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [intent.conversation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        reopened.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::Committed { .. }
    ));
    assert!(reopened.cancel_queue_preview(&intent).is_err());
}

#[test]
fn ordinary_bottom_layer_still_rejects_compact() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    assert!(
        input_queue::cancel_preview_checked(
            &mut f.db.lock().unwrap(),
            f.conversation,
            intent.request_id,
            intent.expected_queue_revision,
            f.input,
            prepare.request_id,
            |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(f.q(), before);
}

#[test]
fn admitted_compact_keeps_claim_and_requires_real_settlement() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&prepare);
    f.events(
        &state.record,
        vec![EventPayload::TurnStarted {
            task_kind: TaskKind::Compact,
            turn_id: state.record.turn_id,
            context_snapshot_id: state.record.context_snapshot_id,
            model: state.record.input.model.clone(),
        }],
    );
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(f.q(), before);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
}

#[test]
fn stale_queue_cancel_preserves_claim_then_exact_current_revision_can_cancel() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    f.edit(QueueEdit::Add(QueuedInput {
        id: Uuid::new_v4(),
        text: "Tail".into(),
        user_input: vec![],
        origin: None,
    }));
    let before = f.q();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(f.q(), before);
    let fresh = CancelPreviewIntent {
        request_id: Uuid::new_v4(),
        expected_queue_revision: before.revision(),
        ..intent
    };
    let (released, _) = f.db.cancel_queue_preview(&fresh).unwrap();
    assert_eq!(released.inputs(), before.inputs());
    assert!(released.paused());
}

#[test]
fn missing_typed_prepare_proof_cannot_release_compact_claim() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    f.db.lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [prepare.request_id.to_string()],
        )
        .unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(f.q(), before);
}

#[test]
fn foreign_input_and_manuscript_cannot_release_compact_claim() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let original = cancellation(&f, &prepare);
    let before = f.q();
    for intent in [
        CancelPreviewIntent {
            input_id: Uuid::new_v4(),
            ..original.clone()
        },
        CancelPreviewIntent {
            manuscript_id: Uuid::new_v4(),
            ..original
        },
    ] {
        assert!(f.db.cancel_queue_preview(&intent).is_err());
        assert_eq!(f.q(), before);
    }
}

#[test]
fn sql_failure_rolls_back_queue_pointer_and_both_receipts() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    f.db.lock().unwrap().execute_batch(
        "create temp trigger fail_compact_cancel before insert on data_sql_operation_receipts
         when new.kind='agent.queue.preview.cancel' begin select raise(abort,'synthetic cancel failure'); end;",
    ).unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(f.q(), before);
    assert!(
        matches!(f.read(Some(prepare.request_id)), Some(QueuedCompactionRead::Compact(state)) if state.current)
    );
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_compact_cancel")
        .unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_ok());
}

#[test]
fn malformed_history_cannot_be_used_as_no_start_proof() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let observer = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
    let snapshot = || {
        observer
            .query_row(
                "select snapshot_json from agent_input_queues where conversation_id=?1",
                [f.conversation.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    let before = snapshot();
    f.db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set durable_count=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert!(f.db.lock().is_err());
    assert_eq!(snapshot(), before);
}

#[test]
fn altered_prepare_fingerprint_cannot_release_claim() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    f.db.lock()
        .unwrap()
        .execute(
            "update data_sql_operation_receipts set fingerprint=zeroblob(32) where operation_id=?1",
            [prepare.request_id.to_string()],
        )
        .unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(f.q(), before);
}

#[test]
fn unrelated_history_advance_does_not_require_old_live_w_for_safe_cancel() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&prepare);
    let mut other = state.record.clone();
    other.request_id = Uuid::new_v4();
    other.turn_id = Uuid::new_v4();
    f.events(
        &other,
        vec![
            EventPayload::TurnStarted {
                task_kind: TaskKind::Regular,
                turn_id: other.turn_id,
                context_snapshot_id: other.context_snapshot_id,
                model: other.input.model.clone(),
            },
            EventPayload::TurnFinished {
                turn_id: other.turn_id,
                outcome: TurnOutcome::Completed,
            },
        ],
    );
    assert!(
        f.db.cancel_queue_preview(&cancellation(&f, &prepare))
            .is_ok()
    );
}

#[test]
fn archived_conversation_cannot_release_unstarted_claim() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    f.db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    let sql = f.db.lock().unwrap();
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
}

#[test]
fn accepted_unmaterialized_start_is_preserved_by_inspection_and_projects_before_cancel() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    let state = f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let sql = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.conversation),
        1,
        0,
        prepare.request_id,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Compact,
            turn_id: state.record.turn_id,
            context_snapshot_id: state.record.context_snapshot_id,
            model: state.record.input.model.clone(),
        },
    )
    .unwrap();
    let receipt =
        f.db.write(
            JournalCommand::new(
                ConversationId::from_uuid(f.conversation),
                Uuid::new_v4(),
                0,
                vec![event],
                None,
                1_790_000_000_000,
            )
            .unwrap(),
            WriteOptions::default(),
        )
        .unwrap();
    assert!(!receipt.materialized);
    let count = || {
        sql.query_row(
            "select durable_count from agent_conversations where id=?1",
            [f.conversation.to_string()],
            |row| row.get::<_, u64>(0),
        )
        .unwrap()
    };
    assert_eq!(count(), 0);
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    assert_eq!(count(), 0);
    assert!(f.db.cancel_queue_preview(&intent).is_err());
    assert_eq!(count(), 1);
    assert_eq!(f.q().preparing(), Some(f.input));
    assert_eq!(
        sql.query_row(
            "select count(*) from data_sql_operation_receipts where operation_id=?1",
            [intent.request_id.to_string()],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn real_deferred_compact_cancel_commit_failure_reopens_same_intent_without_partial_release() {
    let f = Fixture::new();
    let prepare = f.intent(Uuid::new_v4(), None);
    f.prepare(&prepare);
    let intent = cancellation(&f, &prepare);
    let before = f.q();
    f.db.lock().unwrap().execute_batch("create table compact_cancel_parent(id text primary key); create table compact_cancel_child(id text primary key,parent_id text references compact_cancel_parent(id) deferrable initially deferred); create trigger fail_compact_cancel_commit after insert on data_sql_operation_receipts when new.kind='agent.queue.preview.cancel' begin insert into compact_cancel_child(id,parent_id) values(new.operation_id,'missing'); end;").unwrap();
    let error = f.db.cancel_queue_preview(&intent).unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    let diagnostic = error.diagnostic().unwrap();
    assert_eq!(diagnostic.operation_id, intent.request_id);
    assert_eq!(diagnostic.phase, "commit");
    assert_eq!(diagnostic.sqlite_extended_code, Some(787));
    assert!(
        matches!(f.db.queue_preview_cancel_status(&intent).unwrap(), QueuePreviewCancelStatus::Unconfirmed(detail)
        if detail.safe_cause == "owner_blocked_receipt_unconfirmed")
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
    for query in [
        "select count(*) from agent_input_queue_receipts where request_id=?1",
        "select count(*) from data_sql_operation_receipts where operation_id=?1",
    ] {
        assert_eq!(
            sql.query_row(query, [intent.request_id.to_string()], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert!(f.db.raw.lock().unwrap().is_autocommit());
    drop(sql);
    let f = f.reopen();
    f.db.lock()
        .unwrap()
        .execute_batch("drop trigger fail_compact_cancel_commit;")
        .unwrap();
    assert!(matches!(
        f.db.queue_preview_cancel_status(&intent).unwrap(),
        QueuePreviewCancelStatus::NotFound
    ));
    let (released, _) = f.db.cancel_queue_preview(&intent).unwrap();
    assert_eq!(released.inputs(), before.inputs());
    assert_eq!(released.revision(), before.revision() + 1);
    assert!(released.paused());
    assert_eq!(released.preparing(), None);
}
