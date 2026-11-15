use super::*;
use crate::agent::repository::{append_batch, create_conversation, read_history};
use research_agent_core::conversation::HistoryQuery;

pub(super) fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 144);
    let connection = Arc::new(Mutex::new(connection));
    (
        connection.clone(),
        SqliteConversationCatalog::new(connection),
    )
}
pub(super) fn id(value: u128) -> ConversationId {
    ConversationId::from_uuid(Uuid::from_u128(value))
}

#[tokio::test]
async fn exact_metadata_reads_archived_history_without_mutations_or_runtime_recovery() {
    let (connection, catalog) = setup();
    let changes = {
        let mut db = connection.lock().unwrap();
        create_conversation(&mut db, id(1), "Archived").unwrap();
        append_batch(&mut db, id(1), Uuid::new_v4(), 0, &["opaque saved history"]).unwrap();
        db.execute(
            "update agent_conversations set archived = 1 where id = ?1",
            [id(1).as_uuid().to_string()],
        )
        .unwrap();
        db.total_changes()
    };
    let first = catalog.read(id(1), None).await.unwrap();
    assert!(first.archived);
    assert_eq!(first.durable_count, 1);
    assert_eq!(catalog.read(id(1), None).await.unwrap(), first);
    assert_eq!(
        catalog.read(id(0), None).await.unwrap_err(),
        CatalogError::InvalidInput
    );
    assert_eq!(
        catalog.read(id(99), None).await.unwrap_err(),
        CatalogError::NotFound
    );
    let mut db = connection.lock().unwrap();
    assert_eq!(db.total_changes(), changes);
    assert_eq!(
        read_history::<String>(&mut db, HistoryQuery::new(id(1), 0, 10).unwrap())
            .unwrap()
            .items,
        ["opaque saved history"]
    );
}

#[tokio::test]
async fn keyset_pages_are_stable_across_rename_and_separate_archived_items() {
    let (connection, catalog) = setup();
    {
        let mut db = connection.lock().unwrap();
        for n in 1..=5 {
            create_conversation(&mut db, id(n), &format!("Chat {n}")).unwrap();
        }
        db.execute(
            "update agent_conversations set created_at = '2026-09-08T00:00:00.000Z'",
            [],
        )
        .unwrap();
        db.execute(
            "update agent_conversations set archived = 1 where id = ?1",
            [id(3).as_uuid().to_string()],
        )
        .unwrap();
    }
    let first = catalog
        .list(CatalogQuery {
            internal: false,
            binding: None,
            archived: false,
            limit: 2,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(
        first.conversations.iter().map(|c| c.id).collect::<Vec<_>>(),
        [id(5), id(4)]
    );
    catalog
        .rename_legacy_for_tests(id(5), "Chat 5".into(), "Updated".into())
        .await
        .unwrap();
    let second = catalog
        .list(CatalogQuery {
            internal: false,
            binding: None,
            archived: false,
            limit: 2,
            before: first.next,
        })
        .await
        .unwrap();
    assert_eq!(
        second
            .conversations
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [id(2), id(1)]
    );
    assert!(second.next.is_none());
    let archived = catalog
        .list(CatalogQuery {
            internal: false,
            binding: None,
            archived: true,
            limit: 2,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(archived.conversations[0].id, id(3));
    assert!(matches!(
        catalog
            .list(CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 0,
                before: None
            })
            .await,
        Err(CatalogError::InvalidInput)
    ));
}

#[tokio::test]
async fn rename_conflicts_are_explicit_and_event_history_is_unchanged() {
    let (connection, catalog) = setup();
    {
        let mut db = connection.lock().unwrap();
        create_conversation(&mut db, id(1), "Old").unwrap();
        append_batch(&mut db, id(1), Uuid::new_v4(), 0, &["immutable event"]).unwrap();
    }
    let renamed = catalog
        .rename_legacy_for_tests(id(1), "Old".into(), " 新标题 ".into())
        .await
        .unwrap();
    assert_eq!(renamed.title, "新标题");
    assert_eq!(renamed.durable_count, 1);
    assert_eq!(
        catalog
            .rename_legacy_for_tests(id(1), "Old".into(), "新标题".into())
            .await
            .unwrap(),
        renamed
    );
    assert!(matches!(
        catalog
            .rename_legacy_for_tests(id(1), "Old".into(), "Stale edit".into())
            .await,
        Err(CatalogError::Conflict)
    ));
    assert!(matches!(
        catalog
            .rename_legacy_for_tests(id(1), "新标题".into(), " ".into())
            .await,
        Err(CatalogError::InvalidInput)
    ));
    assert!(matches!(
        catalog
            .rename_legacy_for_tests(id(2), "Old".into(), "New".into())
            .await,
        Err(CatalogError::NotFound)
    ));
    let history = read_history::<String>(
        &mut connection.lock().unwrap(),
        HistoryQuery::new(id(1), 0, 10).unwrap(),
    )
    .unwrap();
    assert_eq!(history.items, ["immutable event"]);
}

#[tokio::test]
async fn bindings_are_idempotent_and_filter_without_changing_history() {
    let (connection, catalog) = setup();
    {
        let mut db = connection.lock().unwrap();
        create_conversation(&mut db, id(1), "First").unwrap();
        create_conversation(&mut db, id(2), "Second").unwrap();
        append_batch(&mut db, id(1), Uuid::new_v4(), 0, &["saved event"]).unwrap();
        db.execute("insert into research_projects(id,title,summary,created_at) values ('project','Project','','now')", []).unwrap();
        for resource in ["one", "two"] {
            db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values (?1,'project','Manuscript','now','now')", [resource]).unwrap();
        }
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('one','Note','now','now')", []).unwrap();
    }
    let binding = ConversationBinding {
        kind: BindingKind::Manuscript,
        resource_id: "one".into(),
    };
    catalog
        .bind_legacy_for_tests(id(1), binding.clone())
        .await
        .unwrap();
    catalog
        .bind_legacy_for_tests(id(1), binding.clone())
        .await
        .unwrap();
    assert_eq!(
        catalog.read(id(1), Some(binding.clone())).await.unwrap().id,
        id(1)
    );
    assert_eq!(
        catalog
            .read(id(2), Some(binding.clone()))
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    assert_eq!(
        catalog
            .read(
                id(1),
                Some(ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "one".into()
                })
            )
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    assert_eq!(
        catalog
            .read(
                id(1),
                Some(ConversationBinding {
                    kind: BindingKind::Manuscript,
                    resource_id: "".into()
                })
            )
            .await
            .unwrap_err(),
        CatalogError::InvalidInput
    );
    catalog
        .bind_legacy_for_tests(
            id(2),
            ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "one".into(),
            },
        )
        .await
        .unwrap();
    catalog
        .bind_legacy_for_tests(
            id(2),
            ConversationBinding {
                kind: BindingKind::Manuscript,
                resource_id: "two".into(),
            },
        )
        .await
        .unwrap();
    let page = catalog
        .list(CatalogQuery {
            internal: false,
            binding: Some(binding),
            archived: false,
            limit: 10,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].id, id(1));
    assert_eq!(page.conversations[0].durable_count, 1);
    assert!(matches!(
        catalog
            .bind_legacy_for_tests(
                id(2),
                ConversationBinding {
                    kind: BindingKind::Paper,
                    resource_id: "missing".into()
                }
            )
            .await,
        Err(CatalogError::NotFound)
    ));
    assert!(matches!(
        catalog
            .bind_legacy_for_tests(
                id(99),
                ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "one".into()
                }
            )
            .await,
        Err(CatalogError::NotFound)
    ));
    let mut db = connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_conversation_bindings",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    // Deleting a source must not cascade into conversation history.
    db.execute("delete from manuscripts where id = 'one'", [])
        .unwrap();
    assert_eq!(
        read_history::<String>(&mut db, HistoryQuery::new(id(1), 0, 10).unwrap())
            .unwrap()
            .items,
        ["saved event"]
    );
}
