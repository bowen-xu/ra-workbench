use super::*;
use crate::agent::model::responses_metadata::ResponsesInputKind;

#[tokio::test]
async fn authority_migration_preserves_old_checkpoint_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("authority-upgrade.sqlite");
    let scope = initialize_through(&path, 139);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 16384).unwrap();
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), input())
        .await
        .unwrap();
    store.fixture_preserve(receipt(&scope)).await.unwrap();
    drop(store);
    let snapshot = |db: &Connection| -> (u32, String, String) {
        db.query_row(
            "select schema_version,checkpoint_json,updated_at from agent_responses_checkpoints",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    };
    let before = snapshot(&db.lock().unwrap());
    crate::storage::migrate(&mut db.lock().unwrap()).unwrap();
    assert_eq!(snapshot(&db.lock().unwrap()), before);
    assert_eq!(before.0, 2);
    drop(db);
    let reopened = SqliteResponsesState::new(connection(&path), scope.clone(), 16384).unwrap();
    let wire = reopened
        .fixture_prepare_with_authority(
            Uuid::from_u128(1),
            scope.model,
            next_input(),
            ResponsesInputKind::Scoped {
                authority_items: 0,
                text_compaction: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        wire.iter()
            .filter(|item| item["type"] == "reasoning")
            .count(),
        1
    );
    assert_eq!(wire.last().unwrap()["content"], "continue");
}
