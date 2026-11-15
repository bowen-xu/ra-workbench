//! Cold recovery must distinguish a partial import from an interrupted live turn.
use super::*;

#[tokio::test]
async fn changed_legacy_metadata_retries_using_the_claimed_source() {
    for pending in [false, true] {
        let f = Fixture::new("Legacy title");
        if pending {
            f.store.connection().unwrap().execute_batch(
                "create trigger reject_completion before update of status on agent_paper_reading_history_migrations when new.status='complete' begin select raise(ignore); end;",
            ).unwrap();
        }
        let result = migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy()).await;
        if pending {
            assert!(matches!(result, Err(Error::Unavailable)));
            f.store
                .connection()
                .unwrap()
                .execute_batch("drop trigger reject_completion;")
                .unwrap();
        } else {
            assert!(result.unwrap());
        }
        let id: String = f.store.connection().unwrap().query_row(
            "select conversation_id from agent_paper_reading_history_migrations where legacy_session_id='legacy-session'",
            [], |row| row.get(0),
        ).unwrap();
        let id = Uuid::parse_str(&id).unwrap();
        let before = raw_metadata(&f.store, id);
        let legacy = f.legacy();
        let original_turns = load_turns(&f.store, &legacy.id).unwrap();
        f.store
            .connection()
            .unwrap()
            .execute(
                "update paper_ai_reading_turns set raw_response_json='{}' where id='legacy-turn'",
                [],
            )
            .unwrap();
        let changed_turns = load_turns(&f.store, &legacy.id).unwrap();
        assert_eq!(
            original_turns
                .iter()
                .flat_map(|turn| turn_payloads(&legacy.id, turn))
                .collect::<Vec<_>>(),
            changed_turns
                .iter()
                .flat_map(|turn| turn_payloads(&legacy.id, turn))
                .collect::<Vec<_>>()
        );
        assert!(
            !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
                .await
                .unwrap()
        );
        assert_eq!(raw_metadata(&f.store, id), before);
        f.close();
    }
}

#[tokio::test]
async fn fresh_claim_freezes_source_and_rolls_back_if_source_or_intent_changes() {
    let f = Fixture::new("Legacy title");
    let legacy = f.legacy();
    let turns = load_turns(&f.store, &legacy.id).unwrap();
    let payloads = turns
        .iter()
        .flat_map(|turn| turn_payloads(&legacy.id, turn))
        .collect::<Vec<_>>();
    let id = Uuid::from_u128(713);
    let expected = payloads.len() as i64;
    let mut changed = payloads.clone();
    let text = changed
        .iter_mut()
        .find_map(|payload| match payload {
            EventPayload::TextDelta { text, .. } => Some(text),
            _ => None,
        })
        .unwrap();
    *text = "Changed".into();
    assert!(matches!(
        reserve_mapping(
            &f.store,
            "paper",
            &legacy.id,
            id,
            expected,
            Some((&legacy, &turns, &changed))
        ),
        Err(Error::Corrupt)
    ));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update paper_ai_reading_turns set assistant_message='changed' where id='legacy-turn'",
            [],
        )
        .unwrap();
    assert!(matches!(
        reserve_mapping(
            &f.store,
            "paper",
            &legacy.id,
            id,
            expected,
            Some((&legacy, &turns, &payloads))
        ),
        Err(Error::Conflict)
    ));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update paper_ai_reading_turns set assistant_message='Answer' where id='legacy-turn'",
            [],
        )
        .unwrap();
    f.store.connection().unwrap().execute_batch("create trigger ignore_legacy_intent before insert on agent_legacy_event_intents begin select raise(ignore); end;").unwrap();
    assert!(
        reserve_mapping(
            &f.store,
            "paper",
            &legacy.id,
            id,
            expected,
            Some((&legacy, &turns, &payloads))
        )
        .is_err()
    );
    {
        let db = f.store.connection().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from agent_conversations where id=?1",
                [id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "select count(*) from agent_paper_reading_history_migrations",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger ignore_legacy_intent;")
            .unwrap();
    }
    reserve_mapping(
        &f.store,
        "paper",
        &legacy.id,
        id,
        expected,
        Some((&legacy, &turns, &payloads)),
    )
    .unwrap();
    assert!(
        event_intent::validate_if_present(&f.store, "paper", &legacy.id, id, &payloads).is_ok()
    );
    assert!(matches!(
        event_intent::validate_if_present(&f.store, "paper", &legacy.id, id, &changed),
        Err(Error::Conflict)
    ));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_legacy_event_intents set source_sha256=?1 where conversation_id=?2",
            rusqlite::params!["0".repeat(64), id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        event_intent::validate_if_present(&f.store, "paper", &legacy.id, id, &payloads),
        Err(Error::Corrupt)
    ));
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_legacy_event_intents where conversation_id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        event_intent::validate_if_present(&f.store, "paper", &legacy.id, id, &payloads),
        Err(Error::Corrupt)
    ));
    f.close();
}

#[tokio::test]
async fn fresh_claim_evidence_survives_same_database_and_log_reopen() {
    let f = Fixture::new("Legacy title");
    let legacy = f.legacy();
    let turns = load_turns(&f.store, &legacy.id).unwrap();
    let payloads = turns
        .iter()
        .flat_map(|turn| turn_payloads(&legacy.id, turn))
        .collect::<Vec<_>>();
    let id = Uuid::from_u128(714);
    reserve_mapping(
        &f.store,
        "paper",
        &legacy.id,
        id,
        payloads.len() as i64,
        Some((&legacy, &turns, &payloads)),
    )
    .unwrap();
    let original: (String, String, String) = f.store.connection().unwrap().query_row(
        "select epoch,source_sha256,payloads_sha256 from agent_legacy_event_intents where conversation_id=?1",
        [id.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    let Fixture {
        store,
        catalog,
        sessions,
        root,
    } = f;
    drop(sessions);
    drop(catalog);
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    let mut sql = rusqlite::Connection::open(root.path().join("migration.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database, root.path().join("objects"));
    let reopened: (String, String, String) = store.connection().unwrap().query_row(
        "select epoch,source_sha256,payloads_sha256 from agent_legacy_event_intents where conversation_id=?1",
        [id.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(reopened, original);
    assert!(event_intent::validate_if_present(&store, "paper", &legacy.id, id, &payloads).is_ok());
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    drop(root);
}

#[tokio::test]
async fn fresh_import_over_one_batch_preserves_all_original_events() {
    let f = Fixture::new("Legacy title");
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        for ordinal in 1..36 {
            tx.execute(
                "insert into paper_ai_reading_turns(id,session_id,ordinal,user_message,assistant_message,messages_json,provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at)
                 values(?1,'legacy-session',?2,'Question','Answer','[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Answer\"}]','legacy-provider','legacy-model','succeeded',5,4,'2026-01-01','2026-01-01T01:00:00Z')",
                rusqlite::params![format!("legacy-turn-{ordinal}"), ordinal],
            ).unwrap();
        }
        tx.commit().unwrap();
    }
    let payloads = load_turns(&f.store, "legacy-session")
        .unwrap()
        .iter()
        .flat_map(|turn| turn_payloads("legacy-session", turn))
        .collect::<Vec<_>>();
    assert!(payloads.len() > 450);
    assert!(
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    let id: String = f.store.connection().unwrap().query_row(
        "select conversation_id from agent_paper_reading_history_migrations where paper_id='paper' and legacy_session_id='legacy-session' and status='complete'", [], |row| row.get(0),
    ).unwrap();
    let id = Uuid::parse_str(&id).unwrap();
    assert!(journal_matches(&f.store, id, &payloads).unwrap());
    assert!(
        !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    assert!(journal_matches(&f.store, id, &payloads).unwrap());
    f.close();
}

#[tokio::test]
async fn oversized_unicode_legacy_answer_migrates_and_replays_without_text_loss() {
    let f = Fixture::new("Legacy title");
    let answer = format!("{}界🙂", "🦀".repeat(65_535));
    let messages = serde_json::json!([
        {"role":"user","content":"Question"},
        {"role":"assistant","content":answer},
    ])
    .to_string();
    f.store.connection().unwrap().execute(
        "update paper_ai_reading_turns set assistant_message=?1,messages_json=?2 where id='legacy-turn'",
        rusqlite::params![answer, messages],
    ).unwrap();
    let turns = load_turns(&f.store, "legacy-session").unwrap();
    let payloads = turn_payloads("legacy-session", &turns[0]);
    let texts = payloads
        .iter()
        .filter_map(|value| match value {
            EventPayload::TextDelta { text, .. } if text != "Question" => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        texts
            .iter()
            .map(|text| text.chars().count())
            .collect::<Vec<_>>(),
        vec![65_536, 1]
    );
    assert_eq!(texts.concat(), answer);
    assert!(
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    let id: String = f.store.connection().unwrap().query_row(
        "select conversation_id from agent_paper_reading_history_migrations where paper_id='paper' and legacy_session_id='legacy-session' and status='complete'",
        [], |row| row.get(0),
    ).unwrap();
    let id = Uuid::parse_str(&id).unwrap();
    assert!(journal_matches(&f.store, id, &payloads).unwrap());
    assert!(
        !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    assert!(journal_matches(&f.store, id, &payloads).unwrap());
    f.close();
}

#[tokio::test]
async fn partial_import_ending_at_a_turn_boundary_cannot_open_a_normal_writer() {
    let f = Fixture::new("Legacy title");
    f.store.connection().unwrap().execute(
        "insert into paper_ai_reading_turns(id,session_id,ordinal,user_message,assistant_message,messages_json,provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at)
         select 'legacy-turn-2',session_id,1,user_message,assistant_message,messages_json,provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at from paper_ai_reading_turns where id='legacy-turn'", [],
    ).unwrap();
    let conversation = Uuid::from_u128(711);
    let payloads = named_pending(&f, conversation, false).await;
    let first = turn_payloads(
        "legacy-session",
        &load_turns(&f.store, "legacy-session").unwrap()[0],
    );
    assert!(first.len() < payloads.len());
    assert!(matches!(
        first.last(),
        Some(EventPayload::TurnFinished { .. })
    ));
    let id = ConversationId::from_uuid(conversation);
    f.sessions.open(id).await.unwrap();
    f.sessions
        .commit_payloads(id, Uuid::from_u128(712), first.clone())
        .await
        .unwrap();
    f.sessions.release_idle(id).await.unwrap();
    let reopened = AgentSessionService::from_app_store(&f.store).unwrap();
    assert!(
        matches!(reopened.open(id).await, Err(AgentServiceError::LegacyMigrationIncomplete(found)) if found == conversation)
    );
    assert!(
        matches!(reopened.recover(id).await, Err(AgentServiceError::LegacyMigrationIncomplete(found)) if found == conversation)
    );
    let history = reopened.read_since(id, 0, 500).await.unwrap();
    assert_eq!(history.durable_count, first.len() as u64);
    assert!(journal_matches(&f.store, conversation, &first).unwrap());
    drop(reopened);
    f.close();
}

#[tokio::test]
async fn completion_refuses_incomplete_durable_history_and_keeps_recovery_guard() {
    let f = Fixture::new("Legacy title");
    let conversation = Uuid::from_u128(709);
    let payloads = named_pending(&f, conversation, false).await;
    let expected = payloads.len() as i64;
    assert!(expected > 4);
    assert!(mark_complete(&f.store, "paper", "legacy-session", conversation, expected).is_err());
    f.sessions
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    f.sessions
        .commit_payloads(
            ConversationId::from_uuid(conversation),
            Uuid::from_u128(710),
            payloads[..4].to_vec(),
        )
        .await
        .unwrap();
    assert!(mark_complete(&f.store, "paper", "legacy-session", conversation, expected).is_err());
    let state: (String, u64, u64) = f.store.connection().unwrap().query_row(
        "select m.status, c.durable_count, (select count(*) from agent_legacy_import_recovery_guards g where g.conversation_id=m.conversation_id) from agent_paper_reading_history_migrations m join agent_conversations c on c.id=m.conversation_id where m.conversation_id=?1",
        [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, ("pending".to_owned(), 4, 1));
    assert!(journal_matches(&f.store, conversation, &payloads[..4]).unwrap());
    f.close();
}

#[tokio::test]
async fn ignored_completion_does_not_report_success_and_the_same_import_can_retry() {
    let f = Fixture::new("Legacy title");
    f.store.connection().unwrap().execute_batch(
        "create trigger test_ignore_legacy_completion before update of status on agent_paper_reading_history_migrations when new.status='complete' begin select raise(ignore); end;",
    ).unwrap();
    assert!(matches!(
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy()).await,
        Err(Error::Unavailable)
    ));
    let (conversation, state, guard): (String, String, u64) = f.store.connection().unwrap().query_row(
        "select m.conversation_id, m.status, (select count(*) from agent_legacy_import_recovery_guards g where g.conversation_id=m.conversation_id) from agent_paper_reading_history_migrations m where m.paper_id='paper' and m.legacy_session_id='legacy-session'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((state.as_str(), guard), ("pending", 1));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger test_ignore_legacy_completion;")
        .unwrap();
    assert!(
        !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    let (after, state, guard): (String, String, u64) = f.store.connection().unwrap().query_row(
        "select m.conversation_id, m.status, (select count(*) from agent_legacy_import_recovery_guards g where g.conversation_id=m.conversation_id) from agent_paper_reading_history_migrations m where m.paper_id='paper' and m.legacy_session_id='legacy-session'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(after, conversation);
    assert_eq!((state.as_str(), guard), ("complete", 0));
    f.close();
}

#[tokio::test]
async fn completion_requires_the_same_pending_claim_and_an_actual_update() {
    let f = Fixture::new("Legacy title");
    let conversation = Uuid::from_u128(707);
    let payloads = named_pending(&f, conversation, true).await;
    let expected = payloads.len() as i64;
    assert!(
        mark_complete(
            &f.store,
            "paper",
            "legacy-session",
            Uuid::from_u128(708),
            expected
        )
        .is_err()
    );
    assert!(
        mark_complete(
            &f.store,
            "paper",
            "legacy-session",
            conversation,
            expected + 1
        )
        .is_err()
    );
    {
        let db = f.store.connection().unwrap();
        db.execute_batch("create trigger test_ignore_legacy_completion before update of status on agent_paper_reading_history_migrations when new.status='complete' begin select raise(ignore); end;").unwrap();
    }
    assert!(mark_complete(&f.store, "paper", "legacy-session", conversation, expected).is_err());
    {
        let db = f.store.connection().unwrap();
        let state: (String, u64) = db.query_row(
            "select status, (select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1) from agent_paper_reading_history_migrations where conversation_id=?1",
            [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("pending".to_owned(), 1));
        db.execute_batch("drop trigger test_ignore_legacy_completion;")
            .unwrap();
    }
    mark_complete(&f.store, "paper", "legacy-session", conversation, expected).unwrap();
    assert!(mark_complete(&f.store, "paper", "legacy-session", conversation, expected).is_err());
    {
        let db = f.store.connection().unwrap();
        let state: (String, u64) = db.query_row(
            "select status, (select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1) from agent_paper_reading_history_migrations where conversation_id=?1",
            [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("complete".to_owned(), 0));
    }
    assert!(journal_matches(&f.store, conversation, &payloads).unwrap());
    f.close();
}

#[tokio::test]
async fn partial_migration_is_preserved_across_cold_reopen_and_normal_recovery() {
    let f = Fixture::new("Legacy title");
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        for ordinal in 1..36 {
            tx.execute(
                "insert into paper_ai_reading_turns(id,session_id,ordinal,user_message,assistant_message,messages_json,provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at)
                 values(?1,'legacy-session',?2,'Question','Answer','[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Answer\"}]','legacy-provider','legacy-model','succeeded',5,4,'2026-01-01','2026-01-01T01:00:00Z')",
                rusqlite::params![format!("legacy-turn-{ordinal}"), ordinal],
            ).unwrap();
        }
        tx.commit().unwrap();
    }
    let conversation = Uuid::from_u128(703);
    let payloads = named_pending(&f, conversation, false).await;
    assert!(payloads.len() > 450);
    assert!(!matches!(payloads[449], EventPayload::TurnFinished { .. }));
    f.sessions
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    f.sessions
        .commit_payloads(
            ConversationId::from_uuid(conversation),
            Uuid::from_u128(704),
            payloads[..450].to_vec(),
        )
        .await
        .unwrap();
    assert!(journal_matches(&f.store, conversation, &payloads[..450]).unwrap());
    // The old mapping freezes only a count. A later legacy edit can change the
    // uncommitted suffix without changing either that count or the stored prefix.
    f.store.connection().unwrap().execute(
        "update paper_ai_reading_turns set assistant_message='Changed', messages_json='[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Changed\"}]' where id='legacy-turn-35'",
        [],
    ).unwrap();
    let changed = load_turns(&f.store, "legacy-session")
        .unwrap()
        .iter()
        .flat_map(|turn| turn_payloads("legacy-session", turn))
        .collect::<Vec<_>>();
    assert_eq!(changed.len(), payloads.len());
    assert_eq!(&changed[..450], &payloads[..450]);
    assert_ne!(&changed[450..], &payloads[450..]);
    f.store.connection().unwrap().execute(
        "update paper_ai_reading_turns set assistant_message='Answer', messages_json='[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Answer\"}]' where id='legacy-turn-35'",
        [],
    ).unwrap();
    let before: Vec<String> = {
        let db = f.store.connection().unwrap();
        let mut query = db
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap();
        query
            .query_map([conversation.to_string()], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let Fixture {
        store,
        catalog,
        sessions,
        root,
    } = f;
    drop(sessions);
    drop(catalog);
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    let mut sql = rusqlite::Connection::open(root.path().join("migration.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database, root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
    let sessions = AgentSessionService::from_app_store(&store).unwrap();
    let f = Fixture {
        store,
        catalog,
        sessions,
        root,
    };
    assert!(matches!(
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy()).await,
        Err(Error::Conflict)
    ));
    let after: Vec<String> = {
        let db = f.store.connection().unwrap();
        let mut query = db
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap();
        query
            .query_map([conversation.to_string()], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(after, before);
    let error = f
        .sessions
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AgentServiceError::LegacyMigrationIncomplete(id) if id == conversation)
    );
    let wire = serde_json::to_value(crate::agent::session_commands::SessionCommandError::from(
        error,
    ))
    .unwrap();
    assert_eq!(wire["code"], "agent_legacy_migration_incomplete");
    assert!(
        wire["message"]
            .as_str()
            .unwrap()
            .contains(&conversation.to_string())
    );
    {
        let db = f.store.connection().unwrap();
        // The deployed legacy tables restrict paper deletion until their source
        // turns and session have first been removed.
        db.execute(
            "delete from paper_ai_reading_turns where session_id='legacy-session'",
            [],
        )
        .unwrap();
        db.execute(
            "delete from paper_ai_reading_sessions where id='legacy-session'",
            [],
        )
        .unwrap();
        db.execute("delete from bibliographic_items where id='paper'", [])
            .unwrap();
        assert_eq!(db.query_row("select count(*) from agent_paper_reading_history_migrations where conversation_id=?1", [conversation.to_string()], |r| r.get::<_,u64>(0)).unwrap(), 0);
        assert_eq!(
            db.query_row(
                "select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1",
                [conversation.to_string()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    }
    assert!(
        matches!(f.sessions.open(ConversationId::from_uuid(conversation)).await,
        Err(AgentServiceError::LegacyMigrationIncomplete(id)) if id == conversation)
    );
    assert!(
        matches!(f.sessions.recover(ConversationId::from_uuid(conversation)).await,
        Err(AgentServiceError::LegacyMigrationIncomplete(id)) if id == conversation)
    );
    assert!(journal_matches(&f.store, conversation, &payloads[..450]).unwrap());
    let history = f
        .sessions
        .read_since(ConversationId::from_uuid(conversation), 0, 500)
        .await
        .unwrap();
    assert_eq!(history.durable_count, 450);
    f.close();
}

#[tokio::test]
async fn ordinary_recovered_turn_is_still_interrupted() {
    let f = Fixture::new("Legacy title");
    let conversation = ConversationId::from_uuid(Uuid::from_u128(705));
    let turns = load_turns(&f.store, "legacy-session").unwrap();
    let payloads = turn_payloads("legacy-session", &turns[0]);
    let sessions = AgentSessionService::from_app_store(&f.store).unwrap();
    sessions.create(conversation).await.unwrap();
    sessions
        .commit_payloads(conversation, Uuid::from_u128(706), payloads[..4].to_vec())
        .await
        .unwrap();
    drop(sessions);
    let reopened = AgentSessionService::from_app_store(&f.store).unwrap();
    let projection = reopened.open(conversation).await.unwrap();
    assert_eq!(projection.seq(), 5);
    let history = reopened.read_since(conversation, 4, 1).await.unwrap();
    assert!(matches!(
        history.items[0].payload(),
        EventPayload::TurnFinished {
            outcome: research_agent_core::protocol::TurnOutcome::Interrupted,
            ..
        }
    ));
    drop(reopened);
    f.close();
}
