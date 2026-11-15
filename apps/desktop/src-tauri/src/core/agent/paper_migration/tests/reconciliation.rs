use super::*;
use research_agent_core::protocol::TurnOutcome;

#[test]
fn migrated_journal_reconciliation_checks_event_content_not_only_count() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 108);
    let conversation = Uuid::from_u128(9);
    connection
        .execute(
            "insert into agent_conversations(id,title,durable_count) values(?1,'Legacy',1)",
            [conversation.to_string()],
        )
        .unwrap();
    let turn = Uuid::from_u128(10);
    let event = EventPayload::TurnFinished {
        turn_id: turn,
        outcome: TurnOutcome::Completed,
    };
    connection
            .execute(
                "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",
                rusqlite::params![conversation.to_string(), serde_json::to_string(&event).unwrap()],
            )
            .unwrap();
    let store = crate::storage::AppStore::for_test(
        connection,
        tempfile::tempdir().unwrap().path().to_owned(),
    );
    assert!(journal_matches(&store, conversation, std::slice::from_ref(&event)).unwrap());
    store
        .connection()
        .unwrap()
        .execute(
            "update agent_items set payload_json=?1 where conversation_id=?2",
            rusqlite::params![
                serde_json::to_string(&EventPayload::TurnFinished {
                    turn_id: turn,
                    outcome: TurnOutcome::Failed,
                })
                .unwrap(),
                conversation.to_string()
            ],
        )
        .unwrap();
    assert!(!journal_matches(&store, conversation, &[event]).unwrap());
}

#[tokio::test]
async fn does_not_migrate_an_active_session_with_a_running_turn() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 108);
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
                 values('legacy-active','paper','[]','[]','legacy-provider','legacy-model',
                    'Active reading','active','2026-01-01','2026-01-02')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_ai_reading_turns(
                    id,session_id,ordinal,user_message,assistant_message,messages_json,
                    provider_id,model_id,status,created_at)
                 values('legacy-running-turn','legacy-active',0,'Continue this paper','',
                    '[{\"role\":\"user\",\"content\":\"Continue this paper\"}]',
                    'legacy-provider','legacy-model','running','2026-01-01')",
            [],
        )
        .unwrap();

    let store = crate::storage::AppStore::for_test(
        connection,
        tempfile::tempdir().unwrap().path().to_owned(),
    );
    let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
    let sessions = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "新对话".into()).unwrap(),
    ));
    let legacy = load_legacy(&store, "paper").unwrap();

    assert!(
        !migrate_session(&store, &catalog, &sessions, "paper", &legacy[0])
            .await
            .unwrap()
    );
    store
        .connection()
        .unwrap()
        .execute(
            "update paper_ai_reading_turns set status='succeeded', assistant_message='Done',
                 finished_at='2026-01-01T01:00:00Z' where id='legacy-running-turn'",
            [],
        )
        .unwrap();
    let active_without_running = load_legacy(&store, "paper").unwrap();
    assert!(
        !migrate_session(
            &store,
            &catalog,
            &sessions,
            "paper",
            &active_without_running[0]
        )
        .await
        .unwrap()
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_paper_reading_history_migrations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_conversations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}
