use super::tests::id;
use super::*;
use research_agent_core::conversation::ConversationForkReader;

fn binding(kind: BindingKind) -> ConversationBinding {
    ConversationBinding {
        kind,
        resource_id: "same-id".into(),
    }
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 144);
    db.execute_batch("insert into note_pages(id,title,created_at,updated_at) values ('same-id','Independent page','now','now'); insert into note_blocks(id,page_id,scope_kind,block_type,content_json,created_at,updated_at) values ('page-block','same-id','page','paragraph','{}','now','now');").unwrap();
    let tx = db.transaction().unwrap();
    crate::notes::insert_reading_fixture(&tx, "same-id", "Retained note");
    tx.commit().unwrap();
    let db = Arc::new(Mutex::new(db));
    (db.clone(), SqliteConversationCatalog::new(db))
}

#[tokio::test]
async fn independent_page_and_reading_note_ids_do_not_share_conversations() {
    let (db, catalog) = setup();
    let reading = binding(BindingKind::ReadingNote);
    let page = binding(BindingKind::Note);
    let first = catalog
        .create_bound_legacy_for_tests(id(1), reading.clone())
        .await
        .unwrap();
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(1), reading.clone())
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(1), page.clone())
            .await
            .unwrap_err(),
        CatalogError::Conflict
    );
    catalog
        .create_bound_legacy_for_tests(id(2), page.clone())
        .await
        .unwrap();
    for (scope, expected) in [(reading.clone(), id(1)), (page.clone(), id(2))] {
        let list = catalog
            .list(CatalogQuery {
                internal: false,
                binding: Some(scope.clone()),
                archived: false,
                limit: 10,
                before: None,
            })
            .await
            .unwrap();
        assert_eq!(list.conversations.len(), 1);
        assert_eq!(list.conversations[0].id, expected);
        assert_eq!(
            catalog.read(expected, Some(scope)).await.unwrap().id,
            expected
        );
    }
    assert_eq!(
        catalog.read(id(1), Some(page)).await.unwrap_err(),
        CatalogError::NotFound
    );
    catalog
        .bind_legacy_for_tests(id(2), reading.clone())
        .await
        .unwrap();
    assert_eq!(catalog.read_origin(id(2), reading).await.unwrap(), None);
    // Metadata associations retain history even if the original resource disappears.
    db.lock()
        .unwrap()
        .execute(
            "delete from reading_note_sources where block_id='same-id'",
            [],
        )
        .unwrap();
    assert!(
        catalog
            .read(id(1), Some(binding(BindingKind::ReadingNote)))
            .await
            .is_ok()
    );
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(id(3), binding(BindingKind::ReadingNote))
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    assert_eq!(
        catalog
            .read_origin(id(1), binding(BindingKind::ReadingNote))
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
}

#[tokio::test]
async fn rejects_page_blocks_and_missing_targets_without_creating_or_binding() {
    let (db, catalog) = setup();
    catalog
        .create_bound_legacy_for_tests(id(1), binding(BindingKind::Note))
        .await
        .unwrap();
    let before = db.lock().unwrap().total_changes();
    for resource_id in ["page-block", "missing"] {
        let scope = ConversationBinding {
            kind: BindingKind::ReadingNote,
            resource_id: resource_id.into(),
        };
        assert_eq!(
            catalog
                .create_bound_legacy_for_tests(id(2), scope.clone())
                .await
                .unwrap_err(),
            CatalogError::NotFound
        );
        assert_eq!(
            catalog
                .bind_legacy_for_tests(id(1), scope)
                .await
                .unwrap_err(),
            CatalogError::NotFound
        );
    }
    assert_eq!(db.lock().unwrap().total_changes(), before);
}

#[test]
fn migration_preserves_bindings_lineage_and_foreign_key_policies() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 92);
    db.execute_batch("insert into agent_conversations(id,title) values ('parent','Parent'),('child','Child');
        insert into agent_conversation_bindings values ('parent','note','original','original-time');
        insert into agent_conversation_forks values ('child','parent','turn',5,'note','original','fork-time');").unwrap();
    let migration = crate::storage_migrations::MIGRATIONS
        .iter()
        .find(|(v, _)| *v == 93)
        .unwrap()
        .1;
    let tx = db.transaction().unwrap();
    tx.execute_batch(migration).unwrap();
    tx.commit().unwrap();
    let original: (String, String, String, String) = db
        .query_row("select * from agent_conversation_bindings", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(
        original,
        (
            "parent".into(),
            "note".into(),
            "original".into(),
            "original-time".into()
        )
    );
    let lineage: (String, String, String, i64, String, String, String) = db
        .query_row("select * from agent_conversation_forks", [], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .unwrap();
    assert_eq!(
        lineage,
        (
            "child".into(),
            "parent".into(),
            "turn".into(),
            5,
            "note".into(),
            "original".into(),
            "fork-time".into()
        )
    );
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values ('child','readingNote','retained')", []).unwrap();
    assert!(db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values ('child','unknown','retained')", []).is_err());
    assert!(
        db.execute("delete from agent_conversations where id='parent'", [])
            .is_err()
    );
    db.execute("delete from agent_conversations where id='child'", [])
        .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_conversation_forks", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_conversation_bindings",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        !db.prepare("pragma foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}
