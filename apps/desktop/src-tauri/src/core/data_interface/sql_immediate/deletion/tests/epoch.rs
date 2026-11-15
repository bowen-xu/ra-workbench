use super::*;
use research_agent_core::conversation::{CatalogDataFailureKind, ConversationDeletionError};

#[tokio::test]
async fn deletion_epoch_rejects_before_pending_projection_replay_or_preparation() {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind};
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let other = ConversationId::from_uuid(Uuid::new_v4());
    db.raw
        .lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Other stream')",
            [other.as_uuid().to_string()],
        )
        .unwrap();
    let batch = Uuid::new_v4();
    let event = AgentEvent::new(
        other,
        1,
        1,
        batch,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    db.write(
        JournalCommand::new(other, batch, 0, vec![event], None, 1).unwrap(),
        research_data_runtime::WriteOptions::default(),
    )
    .unwrap();
    let before = db.benchmark_snapshot().unwrap();
    assert!(before.accepted_position > before.materialized_position);
    let changes = db.raw.lock().unwrap().total_changes();
    let Err(ConversationDeletionError::Data(cause)) = SqliteConversationCatalog::new(db.clone())
        .delete_in_epoch(request.clone(), Uuid::new_v4())
        .await
    else {
        panic!("foreign data epoch must refuse before preparation");
    };
    assert_eq!(cause.kind, CatalogDataFailureKind::Conflict);
    assert_eq!(cause.diagnostic.phase, "delete-epoch");
    assert_eq!(cause.diagnostic.safe_cause, "data_epoch_changed");
    assert_eq!(cause.diagnostic.operation_id, request.request_id);
    assert_eq!(cause.diagnostic.epoch, Some(before.epoch));
    assert_eq!(db.benchmark_snapshot().unwrap(), before);
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(counts(&db), (2, 0, 0, 0));
}
#[tokio::test]
async fn deletion_epoch_checks_even_existing_receipts_and_nil_never_falls_back() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let epoch = db.benchmark_snapshot().unwrap().epoch;
    let catalog = SqliteConversationCatalog::new(db.clone());
    let first = catalog
        .delete_in_epoch(request.clone(), epoch)
        .await
        .unwrap();
    let proof = db.deletion_status(&request).unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    assert!(
        matches!(catalog.delete_in_epoch(request.clone(),Uuid::new_v4()).await,Err(ConversationDeletionError::Data(cause)) if cause.kind==CatalogDataFailureKind::Conflict)
    );
    assert_eq!(
        catalog.delete_in_epoch(request.clone(), Uuid::nil()).await,
        Err(ConversationDeletionError::Catalog(
            CatalogError::InvalidInput
        ))
    );
    assert_eq!(
        catalog
            .delete_in_epoch(request.clone(), epoch)
            .await
            .unwrap(),
        first
    );
    assert_eq!(db.deletion_status(&request).unwrap(), proof);
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
}
