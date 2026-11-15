use super::tests::{id, setup};
use super::*;

fn binding(resource: &str) -> ConversationBinding {
    ConversationBinding {
        kind: BindingKind::Note,
        resource_id: resource.into(),
    }
}

#[tokio::test]
async fn atomic_creation_replays_without_resetting_metadata_or_history() {
    let (connection, catalog) = setup();
    connection.lock().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values ('one','Note','now','now'), ('two','Note','now','now')", []).unwrap();
    let (first, second) = tokio::join!(
        catalog.create_bound_legacy_for_tests(id(1), binding("one")),
        catalog.create_bound_legacy_for_tests(id(1), binding("one"))
    );
    assert_eq!(first.unwrap(), second.unwrap());
    {
        let mut db = connection.lock().unwrap();
        crate::agent::repository::append_batch(
            &mut db,
            id(1),
            Uuid::new_v4(),
            0,
            &["unchanged event"],
        )
        .unwrap();
        db.execute(
            "update agent_conversations set title = 'Renamed', archived = 1 where id = ?1",
            [id(1).as_uuid().to_string()],
        )
        .unwrap();
    }
    let changes = connection.lock().unwrap().total_changes();
    let replay = catalog
        .create_bound_legacy_for_tests(id(1), binding("one"))
        .await
        .unwrap();
    assert_eq!(replay.title, "Renamed");
    assert!(replay.archived);
    assert_eq!(replay.durable_count, 1);
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(1), binding("two"))
            .await
            .unwrap_err(),
        CatalogError::Conflict
    );
    assert_eq!(connection.lock().unwrap().total_changes(), changes);
}

#[tokio::test]
async fn binding_failure_rolls_back_creation_and_existing_unbound_ids_are_not_adopted() {
    let (connection, catalog) = setup();
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(0), binding("one"))
            .await
            .unwrap_err(),
        CatalogError::InvalidInput
    );
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(1), binding("missing"))
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    {
        let mut db = connection.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('one','Note','now','now')", []).unwrap();
        crate::agent::repository::create_conversation(&mut db, id(2), "Existing").unwrap();
        db.execute_batch("create trigger reject_binding before insert on agent_conversation_bindings begin select raise(abort, 'synthetic binding failure'); end;").unwrap();
    }
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(2), binding("one"))
            .await
            .unwrap_err(),
        CatalogError::Conflict
    );
    assert!(
        catalog
            .create_bound_legacy_for_tests(id(1), binding("one"))
            .await
            .is_err()
    );
    assert_eq!(
        catalog.read(id(1), None).await.unwrap_err(),
        CatalogError::NotFound
    );
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger reject_binding;")
        .unwrap();
    assert!(
        catalog
            .create_bound_legacy_for_tests(id(1), binding("one"))
            .await
            .is_ok()
    );
}
