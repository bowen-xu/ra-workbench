use super::*;
use research_agent_core::conversation::ConversationForkReader;

#[tokio::test]
async fn reads_immediate_origin_and_current_parent_metadata_without_writes() {
    let (db, catalog) = setup();
    let binding = request(2).binding;
    assert_eq!(
        catalog.read_origin(id(1), binding.clone()).await.unwrap(),
        None
    );
    catalog.fork_legacy_for_tests(request(2)).await.unwrap();
    let mut grandchild = request(3);
    grandchild.parent_id = id(2);
    catalog.fork_legacy_for_tests(grandchild).await.unwrap();
    catalog
        .rename_legacy_for_tests(id(2), "研究讨论 · 分支".into(), "Parent renamed".into())
        .await
        .unwrap();
    catalog.archive_legacy_for_tests(id(2), true).await.unwrap();
    let before = db.lock().unwrap().total_changes();
    let reopened = SqliteConversationCatalog::new(db.clone());
    let origin = reopened
        .read_origin(id(3), binding.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(origin.parent.id, id(2));
    assert_eq!(origin.parent.title, "Parent renamed");
    assert!(origin.parent.archived);
    assert_eq!(origin.through_turn_id, Uuid::from_u128(10));
    assert_eq!(origin.parent_end_seq, 5);
    let first = reopened
        .read_origin(id(2), binding.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.parent.id, id(1));
    assert_eq!(first.parent.durable_count, 6); // The parent's later active tail is not the boundary.
    assert_eq!(first.parent_end_seq, 5);
    assert_eq!(db.lock().unwrap().total_changes(), before);
    assert_eq!(
        reopened
            .read_origin(id(99), binding.clone())
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    let wrong = ConversationBinding {
        resource_id: "other".into(),
        ..binding.clone()
    };
    assert_eq!(
        reopened.read_origin(id(3), wrong).await.unwrap_err(),
        CatalogError::NotFound
    );
    assert_eq!(
        reopened
            .read_origin(ConversationId::from_uuid(Uuid::nil()), binding)
            .await
            .unwrap_err(),
        CatalogError::InvalidInput
    );
}

#[tokio::test]
async fn rejects_corrupt_boundaries_and_does_not_expose_parent_in_a_new_host_binding() {
    let (db, catalog) = setup();
    catalog.fork_legacy_for_tests(request(2)).await.unwrap();
    let child = id(2).as_uuid().to_string();
    {
        let connection = db.lock().unwrap();
        connection.execute("insert into note_pages(id,title,created_at,updated_at) values ('other','Other','now','now')", []).unwrap();
    }
    let extra = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "other".into(),
    };
    catalog
        .bind_legacy_for_tests(id(2), extra.clone())
        .await
        .unwrap();
    assert_eq!(
        catalog.read_origin(id(2), extra).await.unwrap_err(),
        CatalogError::NotFound
    );
    for end in [4, 6, 99] {
        db.lock()
            .unwrap()
            .execute(
                "update agent_conversation_forks set parent_end_seq=?2 where child_id=?1",
                params![child, end],
            )
            .unwrap();
        assert_eq!(
            catalog
                .read_origin(id(2), request(2).binding)
                .await
                .unwrap_err(),
            CatalogError::Corrupt
        );
    }
    db.lock()
        .unwrap()
        .execute(
            "update agent_conversation_forks set parent_end_seq=5 where child_id=?1",
            [&child],
        )
        .unwrap();
    let parent = id(1).as_uuid().to_string();
    let original: String = db
        .lock()
        .unwrap()
        .query_row(
            "select payload_json from agent_items where conversation_id=?1 and ordinal=4",
            [&parent],
            |row| row.get(0),
        )
        .unwrap();
    let oversized = serde_json::json!({"padding":"x".repeat(4096)}).to_string();
    db.lock()
        .unwrap()
        .execute(
            "update agent_items set payload_json=?2 where conversation_id=?1 and ordinal=4",
            params![parent, oversized],
        )
        .unwrap();
    assert_eq!(
        catalog
            .read_origin(id(2), request(2).binding)
            .await
            .unwrap_err(),
        CatalogError::Corrupt
    );
    db.lock()
        .unwrap()
        .execute(
            "update agent_items set payload_json=?2 where conversation_id=?1 and ordinal=4",
            params![parent, original],
        )
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [&parent],
        )
        .unwrap();
    assert_eq!(
        catalog
            .read_origin(id(2), request(2).binding)
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
}
