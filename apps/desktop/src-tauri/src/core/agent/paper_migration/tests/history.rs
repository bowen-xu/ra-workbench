use super::*;

#[tokio::test]
async fn migrates_a_legacy_session_into_the_shared_journal_once() {
    let database_root = tempfile::tempdir().unwrap();
    let database_path = database_root.path().join("agent.sqlite");
    let mut connection = rusqlite::Connection::open(&database_path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 217);
    connection
        .execute(
            "insert into bibliographic_items(id,item_type,created_at,updated_at)
                 values('paper','document','now','now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at)
                 values('legacy-provider',1,'',60000,'now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_ai_reading_sessions(
                    id,paper_id,attachment_ids_json,parse_run_ids_json,provider_id,model_id,
                    title,status,created_at,updated_at)
                 values('legacy-session','paper','[]','[]','legacy-provider','legacy-model',
                    'Old reading','archived','2026-01-01','2026-01-02')",
            [],
        )
        .unwrap();
    connection
            .execute(
                "insert into paper_ai_reading_turns(
                    id,session_id,ordinal,user_message,assistant_message,messages_json,
                    provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at)
                 values('legacy-turn','legacy-session',0,'What is this?','It is a paper.',
                    '[{\"role\":\"user\",\"content\":\"What is this?\"},
                      {\"role\":\"assistant\",\"content\":\"It is a paper.\"}]',
                    'legacy-provider','legacy-model','succeeded',5,4,'2026-01-01','2026-01-01T01:00:00Z')",
                [],
            )
            .unwrap();
    connection
            .execute(
                "insert into paper_ai_reading_turns(
                    id,session_id,ordinal,user_message,assistant_message,messages_json,
                    provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at)
                 values('legacy-turn-2','legacy-session',1,'Why does it matter?','Because it adds evidence.',
                    '[{\"role\":\"user\",\"content\":\"Why does it matter?\"},
                      {\"role\":\"assistant\",\"content\":\"Because it adds evidence.\"}]',
                    'legacy-provider','legacy-model','succeeded',6,5,
                    '2026-01-01T02:00:00Z','2026-01-01T03:00:00Z')",
                [],
            )
            .unwrap();
    let store = crate::storage::AppStore::for_test_database(
        crate::data_interface::Database::open(connection, &database_root.path().join("data-log"))
            .unwrap(),
        database_root.path().join("objects"),
    );
    let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
    let sessions = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "新对话".into()).unwrap(),
    ));
    let legacy = load_legacy(&store, "paper").unwrap();

    let migrated = migrate_session(&store, &catalog, &sessions, "paper", &legacy[0])
        .await
        .unwrap();
    assert!(migrated);

    let mapping: (String, String, i64) = store
        .connection()
        .unwrap()
        .query_row(
            "select conversation_id,status,expected_event_count
                 from agent_paper_reading_history_migrations
                 where paper_id='paper' and legacy_session_id='legacy-session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(mapping.1, "complete");
    assert_eq!(mapping.2, 26);
    let conversation = Uuid::parse_str(&mapping.0).unwrap();
    let summary = catalog
        .read(ConversationId::from_uuid(conversation), None)
        .await
        .unwrap();
    assert_eq!(summary.durable_count, 26);
    assert_eq!(summary.title, "Old reading");

    let legacy_projection: (String, String, String, String, String) = store
        .connection()
        .unwrap()
        .query_row(
            "select paper_id,legacy_session_id,status,title,bootstrap_kind
                 from agent_paper_reading_legacy_sessions
                 where conversation_id=?1",
            [mapping.0.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        legacy_projection,
        (
            "paper".into(),
            "legacy-session".into(),
            "archived".into(),
            "Old reading".into(),
            "fresh_paper".into(),
        )
    );
    let legacy_turns: Vec<(i64, String, String, String, i64, i64)> = {
        let connection = store.connection().unwrap();
        let mut statement = connection
            .prepare(
                "select ordinal,legacy_turn_id,provider_id,model_id,input_tokens,output_tokens
                     from agent_paper_reading_legacy_turns
                     where conversation_id=?1 order by ordinal",
            )
            .unwrap();
        statement
            .query_map([mapping.0.as_str()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        legacy_turns,
        vec![
            (
                0,
                "legacy-turn".into(),
                "legacy-provider".into(),
                "legacy-model".into(),
                5,
                4
            ),
            (
                1,
                "legacy-turn-2".into(),
                "legacy-provider".into(),
                "legacy-model".into(),
                6,
                5
            ),
        ]
    );

    let second = migrate_session(&store, &catalog, &sessions, "paper", &legacy[0])
        .await
        .unwrap();
    assert!(!second);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_items where conversation_id=?1",
                [mapping.0],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        26
    );

    drop(sessions);
    drop(catalog);
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    let reopened = crate::storage::AppStore::for_test_database(
        crate::data_interface::Database::open(
            rusqlite::Connection::open(&database_path).unwrap(),
            &database_root.path().join("data-log"),
        )
        .unwrap(),
        database_root.path().join("objects"),
    );
    let reopened_catalog = SqliteConversationCatalog::new(reopened.shared_agent_connection());
    let reopened_summary = reopened_catalog
        .read(ConversationId::from_uuid(conversation), None)
        .await
        .unwrap();
    assert_eq!(reopened_summary.durable_count, 26);
    assert_eq!(reopened_summary.title, "Old reading");
    assert_eq!(reopened.connection().unwrap().query_row(
            "select count(*) from data_sql_operation_receipts where operation_id=?1 and kind='agent.conversation.rename'",
            [Uuid::new_v5(&conversation, b"agent.paper_migration.rename:v1").to_string()],
            |row| row.get::<_, u64>(0),
        ).unwrap(), 1);
    reopened.shared_agent_connection().shutdown().unwrap();
}
