use super::*;
use research_agent_core::{
    conversation::ConversationSearchPage,
    protocol::{CanonicalItemSource, ModelIdentity, NativeItemData},
};

async fn search(database: &SharedDatabase, term: &str) -> ConversationSearchPage {
    SqliteConversationCatalog::new(database.clone())
        .search(ConversationSearchQuery {
            term: term.into(),
            scope: CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 10,
                before: None,
            },
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn incomplete_interleaved_summary_is_hidden_then_complete_parts_match_independently() {
    let root = tempfile::tempdir().unwrap();
    let database = open_logged(root.path());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let draft = write(
        &database,
        id,
        0,
        vec![
            P::ItemStarted {
                turn_id: turn,
                item_id: item,
                kind: summary_kind(json!([])),
            },
            added(turn, item, "0"),
            delta(turn, item, "0", "ab"),
            added(turn, item, "1"),
            delta(turn, item, "1", "X"),
            delta(turn, item, "0", "cd"),
        ],
        Some("part search"),
        WriteMode::Immediate,
    );
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row(
                "select body from agent_messages where item_id=?1",
                [item.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "abXcd"
    );
    for term in ["bX", "abcd", "ab", "X"] {
        assert!(search(&database, term).await.hits.is_empty(), "{term}");
    }
    let mut expected = draft.clone();
    expected.extend(write(
        &database,
        id,
        draft.len() as u64,
        vec![accepted(
            turn,
            item,
            "abcdX",
            json!([part("0", 0, 4), part("1", 4, 1)]),
        )],
        None,
        WriteMode::Automatic,
    ));
    for (term, excerpt) in [("abcd", "abcd"), ("X", "X")] {
        let result = search(&database, term).await;
        assert_eq!(result.hits.len(), 1);
        let hit = &result.hits[0];
        assert_eq!(hit.item_id, Some(item));
        assert_eq!(hit.turn_id, Some(turn));
        assert_eq!(hit.start_seq, Some(1));
        assert_eq!(hit.excerpt, excerpt);
        assert!(hit.completed);
    }
    for term in ["bX", "dX"] {
        assert!(search(&database, term).await.hits.is_empty(), "{term}");
    }
    assert_eq!(history(&mut database.lock().unwrap(), id), expected);
    database.shutdown().unwrap();
    drop(database);
    let database = open_logged(root.path());
    assert_eq!(search(&database, "X").await.hits[0].excerpt, "X");
    database
        .lock()
        .unwrap()
        .execute(
            "update agent_conversations set title='X title' where id=?1",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    let title = search(&database, "X").await;
    assert_eq!(title.hits.len(), 1);
    assert_eq!(title.hits[0].item_id, None);
    assert_eq!(title.hits[0].excerpt, "X title");
    database.shutdown().unwrap();
}

#[tokio::test]
async fn multiple_matching_parts_return_one_real_item_and_empty_opaque_rows_stay_unsearchable() {
    let root = tempfile::tempdir().unwrap();
    let database = open_logged(root.path());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let first = "first needle";
    let second = "last needle";
    write(
        &database,
        id,
        0,
        vec![accepted(
            turn,
            item,
            &format!("{first}{second}"),
            json!([
                part("-1", 0, first.len()),
                part("9223372036854775807", first.len(), second.len()),
            ]),
        )],
        Some("multi part fixture"),
        WriteMode::Immediate,
    );
    let later = Uuid::new_v4();
    write(
        &database,
        id,
        1,
        vec![accepted(
            turn,
            later,
            "another needle",
            json!([part("9007199254740993", 0, "another needle".len()),]),
        )],
        None,
        WriteMode::Automatic,
    );
    let result = search(&database, "needle").await;
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].item_id, Some(item));
    assert_eq!(result.hits[0].excerpt, first);
    assert!(result.next.is_none());
    let result = search(&database, "last needle").await;
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].item_id, Some(item));
    assert_eq!(result.hits[0].excerpt, second);
    let result = search(&database, "another").await;
    assert_eq!(result.hits[0].item_id, Some(later));
    assert_eq!(result.hits[0].start_seq, Some(2));
    let mut native = NativeItemData::new(
        "responses".into(),
        ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        "reasoning".into(),
        json!({"encrypted_content":"opaque-only-needle"}),
    );
    native.item_id = Some("opaque-fixture".into());
    let empty = Uuid::new_v4();
    write(
        &database,
        id,
        2,
        vec![
            accepted(turn, empty, "", json!([])),
            P::ItemAccepted {
                turn_id: turn,
                item_id: Uuid::new_v4(),
                kind: ItemKind::ProviderNative {
                    item_type: "reasoning".into(),
                },
                text: String::new(),
                phase: None,
                source: Some(CanonicalItemSource {
                    attempt_id: Uuid::new_v4(),
                    ordinal: 0,
                    provider_item_id: Some("opaque-fixture".into()),
                    provider_data_json: Some(native.to_json().unwrap()),
                }),
            },
        ],
        None,
        WriteMode::Automatic,
    );
    assert!(
        search(&database, "opaque-only-needle")
            .await
            .hits
            .is_empty()
    );
    {
        let sql = database.lock().unwrap();
        assert_eq!(
            sql.query_row("select count(*) from agent_reasoning_parts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from agent_search_messages where item_id=?1",
                [empty.to_string()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_fk(&sql);
    }
    database
        .lock()
        .unwrap()
        .execute(
            "delete from agent_conversations where id=?1",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    let sql = database.lock().unwrap();
    assert_eq!(
        sql.query_row("select count(*) from agent_reasoning_parts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_fk(&sql);
    drop(sql);
    database.shutdown().unwrap();
}

#[tokio::test]
async fn real_208_completion_and_pending_suffix_backfill_parts_on_209_210_factory_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("research-assistant.sqlite3");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 208);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    let database = Database::open(sql, &root.path().join("data-log")).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let mut expected = write(
        &database,
        id,
        0,
        vec![accepted(
            turn,
            item,
            "你🙂\0second",
            json!([
                part("-9223372036854775808", 0, "你🙂\0".len()),
                part("7", "你🙂\0".len(), "second".len()),
            ]),
        )],
        Some("208 backfill fixture"),
        WriteMode::Immediate,
    );
    let pending = Uuid::new_v4();
    expected.extend(write(
        &database,
        id,
        1,
        vec![accepted(
            turn,
            pending,
            "pending",
            json!([part("9007199254740993", 0, "pending".len()),]),
        )],
        None,
        WriteMode::Automatic,
    ));
    drop(database);
    let old = Connection::open(&path).unwrap();
    assert_eq!(
        old.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        208
    );
    assert_eq!(
        old.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        old.query_row(
            "select count(*) from sqlite_schema where name='agent_reasoning_parts'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(old);
    let store = super::upgrade::factory(root.path());
    let database = store.shared_agent_connection();
    {
        let mut sql = database.lock().unwrap();
        assert_eq!(
            sql.query_row(
                "select max(version) from schema_migrations where version in (208,209,210)",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            210
        );
        assert_eq!(
            sql.query_row("select count(*) from agent_reasoning_parts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert_eq!(sql.query_row("select summary_index from agent_reasoning_parts where item_id=?1 and part_ordinal=0",[item.to_string()],|r|r.get::<_,String>(0)).unwrap(),"-9223372036854775808");
        assert_eq!(history(&mut sql, id), expected);
        assert_fk(&sql);
    }
    assert_eq!(
        search(&database, "second").await.hits[0].item_id,
        Some(item)
    );
    assert_eq!(search(&database, "second").await.hits[0].excerpt, "second");
    assert_eq!(
        search(&database, "pending").await.hits[0].item_id,
        Some(pending)
    );
    database.shutdown().unwrap();
    drop(database);
    drop(store);
    let store = super::upgrade::factory(root.path());
    let database = store.shared_agent_connection();
    assert_eq!(search(&database, "second").await.hits[0].excerpt, "second");
    assert_eq!(history(&mut database.lock().unwrap(), id), expected);
    database.shutdown().unwrap();
}
