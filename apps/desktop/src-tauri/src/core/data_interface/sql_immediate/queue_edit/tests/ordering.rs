use super::*;
use crate::data_interface::journal::JournalCommand;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind, TurnOutcome},
};

pub(super) fn accept_pending(f: &Fixture) {
    let c = ConversationId::from_uuid(f.intent.conversation_id);
    let r = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let events = [
        EventPayload::TurnStarted {
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "pending".into(),
                revision: None,
            },
            task_kind: TaskKind::Regular,
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(i, payload)| AgentEvent::new(c, i as u64 + 1, 0, r, payload).unwrap())
    .collect();
    assert_eq!(
        f.db.write(
            JournalCommand::new(c, Uuid::new_v4(), 0, events, None, 1_790_000_000_000).unwrap(),
            research_data_runtime::WriteOptions::default()
        )
        .unwrap()
        .position,
        1
    );
}
fn prefix(f: &Fixture) -> u64 {
    f.observer()
        .query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn pure_status_keeps_pending_suffix_and_closed_owner_while_write_orders_after_the_prefix() {
    let f = Fixture::new();
    let (queue, proof) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    accept_pending(&f);
    let fresh = f.next(1, QueueEdit::Pause);
    let changes = f.db.raw.lock().unwrap().total_changes();
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(),QueueEditStatus::Committed{queue:saved,receipt} if saved==queue && receipt.sql_fence==proof.sql_fence)
    );
    assert!(matches!(
        f.db.input_queue_edit_status(&fresh).unwrap(),
        QueueEditStatus::Unconfirmed(_)
    ));
    assert_eq!(prefix(&f), 0);
    assert_eq!(f.db.raw.lock().unwrap().total_changes(), changes);
    let (_, receipt) = committed(f.db.edit_input_queue(&fresh).unwrap());
    assert_eq!(prefix(&f), 1);
    assert_eq!(receipt.sql_fence.position, 1);
    f.db.shutdown().unwrap();
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent).unwrap(),
        QueueEditStatus::Committed { .. }
    ));
    assert!(
        matches!(f.db.input_queue_edit_status(&f.next(2,QueueEdit::Resume)).unwrap(),QueueEditStatus::Unconfirmed(detail) if detail.safe_cause=="owner_closed_receipt_unconfirmed")
    );
    assert!(
        f.db.edit_input_queue(&f.next(2, QueueEdit::Resume))
            .is_err()
    );
}

#[tokio::test]
async fn actual_catalog_delete_returns_deleted_without_fabricated_queue_or_blocking_another_c() {
    use crate::{agent::repository::SqliteConversationCatalog, storage::AppStore};
    use research_agent_core::conversation::{
        ArchiveConversation, ConversationCatalog, ConversationDeletionRequest,
        ConversationDeletionStore,
    };
    let f = Fixture::new();
    let (_, receipt) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    let proof = f.proof();
    let store = AppStore::for_test_database(f.db.clone(), f.root.path().join("data/objects"));
    let catalog = SqliteConversationCatalog::from_app_store(&store);
    let id = ConversationId::from_uuid(f.intent.conversation_id);
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(0),
        })
        .await
        .unwrap();
    catalog
        .delete(ConversationDeletionRequest {
            request_id: Uuid::new_v4(),
            conversation_id: id,
            binding: None,
            expected_title: "Synthetic queue edit".into(),
            expected_durable_count: 0,
            force: false,
        })
        .await
        .unwrap();
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(),QueueEditStatus::Deleted{receipt:old,detail} if old.sql_fence==receipt.sql_fence && detail.sql_fence==Some("0".into()) && detail.safe_cause=="queue_edit_result_deleted")
    );
    assert!(matches!(
        f.db.edit_input_queue(&f.intent),
        Err(QueueEditFailure::Domain(input_queue::Error::Closed))
    ));
    assert_eq!(f.proof(), proof);
    let next = Uuid::new_v4();
    f.db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Unrelated synthetic')",
            [next.to_string()],
        )
        .unwrap();
    let mut intent = f.intent.clone();
    intent.conversation_id = next;
    intent.request_id = Uuid::new_v4();
    if let QueueEdit::Add(input) = &mut intent.edit {
        input.id = Uuid::new_v4();
    }
    assert_eq!(
        committed(f.db.edit_input_queue(&intent).unwrap())
            .0
            .revision(),
        1
    );
    assert_eq!(f.proof(), proof);
}
