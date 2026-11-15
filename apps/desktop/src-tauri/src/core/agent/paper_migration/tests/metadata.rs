use super::*;

#[test]
fn legacy_metadata_projection_is_idempotent_and_keeps_debug_provenance() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 108);
    connection
        .execute(
            "insert into bibliographic_items(id,item_type,created_at,updated_at)
                 values('paper','document','now','now')",
            [],
        )
        .unwrap();
    let conversation = Uuid::from_u128(7);
    connection
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Legacy')",
            [conversation.to_string()],
        )
        .unwrap();
    let store = crate::storage::AppStore::for_test(
        connection,
        tempfile::tempdir().unwrap().path().to_owned(),
    );
    let legacy = LegacySession {
        id: "legacy-session".into(),
        title: "Legacy title".into(),
        status: "archived".into(),
        context_snapshot_id: None,
        attachment_ids_json: "[\"attachment\"]".into(),
        parse_run_ids_json: "[\"parse\"]".into(),
        bootstrap_kind: "structured_history".into(),
        source_workflow_id: Some("workflow".into()),
        source_workflow_revision: Some(3),
        bootstrap_messages_json: "[{\"role\":\"user\",\"content\":\"old\"}]".into(),
        bootstrap_hash: Some("a".repeat(64)),
        created_at: "2026-01-01".into(),
        updated_at: "2026-01-02".into(),
    };
    let turn = LegacyTurn {
        id: "legacy-turn".into(),
        ordinal: 2,
        user_message: "问".into(),
        assistant_message: "答".into(),
        messages_json: "[{\"role\":\"user\",\"content\":\"问\"}]".into(),
        provider_id: "openai".into(),
        model_id: "model".into(),
        reasoning_effort: Some("medium".into()),
        status: "failed".into(),
        input_tokens: Some(11),
        output_tokens: Some(13),
        input_hash: Some("b".repeat(64)),
        output_hash: Some("c".repeat(64)),
        context_window_tokens: Some(4096),
        context_window_source: Some("provider_catalog".into()),
        error_code: Some("timeout".into()),
        error_message: Some("old failure".into()),
        raw_request_json: Some("{\"request\":true}".into()),
        raw_response_json: Some("{\"response\":false}".into()),
        created_at: "2026-01-01".into(),
        finished_at: Some("2026-01-01T01:00:00Z".into()),
    };
    persist_legacy_metadata(&store, "paper", &legacy, &[turn], conversation).unwrap();
    persist_legacy_metadata(
        &store,
        "paper",
        &legacy,
        &[LegacyTurn {
            id: "legacy-turn".into(),
            ordinal: 2,
            user_message: "changed display only".into(),
            assistant_message: "答".into(),
            messages_json: "[]".into(),
            provider_id: "openai".into(),
            model_id: "model".into(),
            reasoning_effort: Some("medium".into()),
            status: "failed".into(),
            input_tokens: Some(11),
            output_tokens: Some(13),
            input_hash: Some("b".repeat(64)),
            output_hash: Some("c".repeat(64)),
            context_window_tokens: Some(4096),
            context_window_source: Some("provider_catalog".into()),
            error_code: Some("timeout".into()),
            error_message: Some("old failure".into()),
            raw_request_json: Some("{\"request\":true}".into()),
            raw_response_json: Some("{\"response\":false}".into()),
            created_at: "2026-01-01".into(),
            finished_at: Some("2026-01-01T01:00:00Z".into()),
        }],
        conversation,
    )
    .unwrap();
    let db = store.connection().unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from agent_paper_reading_legacy_turns where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let raw_response: String = db
            .query_row(
                "select raw_response_json from agent_paper_reading_legacy_turns where conversation_id=?1",
                [conversation.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(raw_response, "{\"response\":false}");
}

#[test]
fn migration_claim_is_unique_and_releasable_after_catalog_failure() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 108);
    connection
        .execute(
            "insert into bibliographic_items(id,item_type,created_at,updated_at)
                 values('paper','document','now','now')",
            [],
        )
        .unwrap();
    let store = crate::storage::AppStore::for_test(
        connection,
        tempfile::tempdir().unwrap().path().to_owned(),
    );
    let first = Uuid::from_u128(71);
    let second = Uuid::from_u128(72);
    reserve_mapping(&store, "paper", "legacy", first, 3, None).unwrap();
    assert!(reserve_mapping(&store, "paper", "legacy", second, 3, None).is_err());
    release_mapping(&store, "paper", "legacy", first).unwrap();
    reserve_mapping(&store, "paper", "legacy", second, 3, None).unwrap();
    let db = store.connection().unwrap();
    let stored: String = db
        .query_row(
            "select conversation_id from agent_paper_reading_history_migrations
                 where paper_id='paper' and legacy_session_id='legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, second.to_string());
}

#[test]
fn release_keeps_mapping_and_binding_when_conversation_is_no_longer_empty() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 108);
    connection.execute("insert into bibliographic_items(id,item_type,created_at,updated_at) values('paper','document','now','now')", []).unwrap();
    let store = crate::storage::AppStore::for_test(
        connection,
        tempfile::tempdir().unwrap().path().to_owned(),
    );
    let conversation = Uuid::from_u128(73);
    reserve_mapping(&store, "paper", "legacy", conversation, 3, None).unwrap();
    let event = EventPayload::TurnFinished {
        turn_id: Uuid::from_u128(74),
        outcome: research_agent_core::protocol::TurnOutcome::Completed,
    };
    crate::agent::repository::append_batch(
        &mut store.connection().unwrap(),
        ConversationId::from_uuid(conversation),
        Uuid::from_u128(75),
        0,
        std::slice::from_ref(&event),
    )
    .unwrap();
    release_mapping(&store, "paper", "legacy", conversation).unwrap();
    let db = store.connection().unwrap();
    let state: (i64, i64, i64) = db.query_row(
        "select (select count(*) from agent_paper_reading_history_migrations where conversation_id=?1),
                (select count(*) from agent_conversation_bindings where conversation_id=?1 and resource_kind='paper' and resource_id='paper'),
                (select durable_count from agent_conversations where id=?1)",
        [conversation.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (1, 1, 1));
    drop(db);
    assert!(journal_matches(&store, conversation, &[event]).unwrap());
}
