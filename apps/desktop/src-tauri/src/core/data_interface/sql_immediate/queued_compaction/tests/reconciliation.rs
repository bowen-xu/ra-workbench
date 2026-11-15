//! A real task-owner reconciliation keeps its own system request envelope.
use super::*;
use crate::agent::{
    repository::SqliteSubmissionStore,
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use std::sync::Arc;

#[tokio::test]
async fn system_reconciliation_envelope_preserves_original_compact_claim_and_can_settle() {
    let f = Fixture::new();
    let intent = f.intent(Uuid::new_v4(), None);
    let prepared = f.prepare(&intent);
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    let submissions = SqliteSubmissionStore::for_app(&f.store);
    let id = ConversationId::from_uuid(f.conversation);
    let admitted = sessions
        .start_compact_submission(
            &submissions,
            id,
            intent.request_id,
            intent.expected_revision,
            |_, _, _| async { Err(research_agent_core::runner::RunError::EventSink) },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("original Compact must start exactly once")
    };
    assert_eq!(owned.turn_id, prepared.record.turn_id);
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let history = sessions.read_since(id, 0, 500).await.unwrap();
    assert_eq!(history.items.len(), 2);
    assert!(
        matches!(history.items[0].payload(),EventPayload::TurnStarted{task_kind:TaskKind::Compact,turn_id,context_snapshot_id,model} if *turn_id==prepared.record.turn_id&&*context_snapshot_id==prepared.record.context_snapshot_id&&*model==prepared.record.input.model)
    );
    assert_eq!(history.items[0].request_id(), intent.request_id);
    assert!(
        matches!(history.items[1].payload(),EventPayload::TurnFinished{turn_id,outcome:TurnOutcome::Interrupted} if *turn_id==prepared.record.turn_id)
    );
    assert_ne!(history.items[1].request_id(), intent.request_id);
    let Some(QueuedCompactionRead::Compact(current)) = f.read(Some(intent.request_id)) else {
        panic!("original queued claim must remain restorable")
    };
    assert_eq!(current.record, prepared.record);
    assert!(current.current);
    assert_eq!(current.outcome, Some(TurnOutcome::Interrupted));
    assert!(current.queue.paused());
    let cancel = crate::data_interface::CancelPreviewIntent {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: Uuid::new_v4(),
        expected_queue_revision: current.queue.revision(),
        input_id: f.input,
        preview_request_id: intent.request_id,
    };
    assert!(matches!(
        f.db.cancel_queue_preview(&cancel),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(f.q(), current.queue);
    let released = f.db.settle_queued_compaction(&f.settle(&intent)).unwrap();
    assert_eq!(released.outcome, TurnOutcome::Interrupted);
    assert!(released.queue.paused());
    assert_eq!(released.queue.preparing(), None);
    assert_eq!(released.queue.inputs()[0].id, f.input);
    assert!(
        matches!(sessions.existing_compaction(&submissions,&prepared.record).await.unwrap(),Some(SubmissionAdmission::AlreadyStarted{turn_id,outcome:Some(TurnOutcome::Interrupted)}) if turn_id==prepared.record.turn_id)
    );
    assert!(
        matches!(f.read(None),Some(QueuedCompactionRead::Compact(state)) if !state.current&&state.record.request_id==intent.request_id&&state.outcome==Some(TurnOutcome::Interrupted))
    );
    sessions.shutdown().await.unwrap();
}

#[test]
fn an_original_start_with_a_foreign_envelope_cannot_authorize_settlement() {
    let f = Fixture::new();
    let intent = f.intent(Uuid::new_v4(), None);
    let prepared = f.prepare(&intent);
    f.terminal(&prepared, TurnOutcome::Completed);
    f.db.lock().unwrap().execute("update agent_items set payload_json=json_set(payload_json,'$.requestId',?2) where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnStarted'",params![f.conversation.to_string(),Uuid::new_v4().to_string()]).unwrap();
    let before: String =
        f.db.lock()
            .unwrap()
            .query_row(
                "select snapshot_json from agent_input_queues where conversation_id=?1",
                [f.conversation.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert!(matches!(
        f.db.settle_queued_compaction(&f.settle(&intent)),
        Err(SqlImmediateFailure::Corrupt(_))
    ));
    let sql = f.db.raw.lock().unwrap();
    let json: String = sql
        .query_row(
            "select snapshot_json from agent_input_queues where conversation_id=?1",
            [f.conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(json, before);
}
