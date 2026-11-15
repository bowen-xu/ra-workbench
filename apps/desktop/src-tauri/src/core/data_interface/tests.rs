use super::*;
use research_agent_core::{
    conversation::{ConversationId, ConversationStore, HistoryQuery, WriterId, WriterLease},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::coordinator::WriteOptions;
use rusqlite::Connection;
use uuid::Uuid;

fn open(path: &std::path::Path) -> SharedDatabase {
    let mut sql = Connection::open(path.join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &path.join("data-log")).unwrap()
}
fn event(id: ConversationId, seq: u64, payload: EventPayload) -> AgentEvent {
    AgentEvent::new(id, seq, seq, Uuid::from_u128(42), payload).unwrap()
}
fn command(
    id: ConversationId,
    expected: u64,
    events: Vec<AgentEvent>,
    title: Option<String>,
) -> journal::JournalCommand {
    journal::JournalCommand::new(
        id,
        Uuid::new_v4(),
        expected,
        events,
        title,
        1_790_000_000_000,
    )
    .unwrap()
}

#[test]
fn saved_turn_admission_checks_legacy_guard_under_the_journal_fence() {
    use research_agent_core::protocol::{ModelIdentity, TaskKind};
    let dir = tempfile::tempdir().unwrap();
    let database = open(dir.path());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    database
        .write(
            command(id, 0, vec![], Some("test".into())),
            WriteOptions::default(),
        )
        .unwrap();
    let request = Uuid::new_v4();
    let turn = Uuid::new_v4();
    {
        let db = database.lock().unwrap();
        db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')", rusqlite::params![id.as_uuid().to_string(), request.to_string(), turn.to_string(), Uuid::new_v4().to_string()]).unwrap();
        db.execute(
            "insert into agent_legacy_import_recovery_guards(conversation_id) values(?1)",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    }
    let payload = EventPayload::TurnStarted {
        task_kind: TaskKind::Regular,
        turn_id: turn,
        context_snapshot_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "test".into(),
            model_id: "test".into(),
            revision: None,
        },
    };
    let start = AgentEvent::new(id, 1, 1, request, payload.clone()).unwrap();
    let original = command(id, 0, vec![start], None);
    let rejected = database
        .write(original.clone(), WriteOptions::default())
        .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("agent_legacy_migration_incomplete")
    );
    assert_eq!(database.benchmark_snapshot().unwrap().accepted_position, 1);
    {
        let db = database.lock().unwrap();
        db.execute(
            "delete from agent_legacy_import_recovery_guards where conversation_id=?1",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    }
    assert!(
        !database
            .write(original.clone(), WriteOptions::default())
            .unwrap()
            .replayed
    );
    {
        let db = database.lock().unwrap();
        db.execute(
            "insert into agent_legacy_import_recovery_guards(conversation_id) values(?1)",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    }
    assert!(
        database
            .write(original, WriteOptions::default())
            .unwrap()
            .replayed
    );
    let legacy = AgentEvent::new(
        id,
        2,
        2,
        Uuid::new_v4(),
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "test".into(),
                model_id: "test".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    assert!(
        !database
            .write(command(id, 1, vec![legacy], None), WriteOptions::default())
            .unwrap()
            .replayed
    );
    database.shutdown().unwrap();
}

#[test]
fn saved_turn_admission_supports_deployed_schema_before_legacy_guard() {
    use research_agent_core::protocol::{ModelIdentity, TaskKind};
    let dir = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(dir.path().join("legacy.sqlite3")).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 215);
    let database = Database::open(sql, &dir.path().join("data-log")).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    database
        .write(
            command(id, 0, vec![], Some("test".into())),
            WriteOptions::default(),
        )
        .unwrap();
    let start = event(
        id,
        1,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "test".into(),
                model_id: "test".into(),
                revision: None,
            },
        },
    );
    assert!(
        !database
            .write(command(id, 0, vec![start], None), WriteOptions::default())
            .unwrap()
            .replayed
    );
    database.shutdown().unwrap();
}

#[tokio::test]
async fn streaming_store_accepts_without_sql_per_fragment_then_folds_body_once() {
    let dir = tempfile::tempdir().unwrap();
    let database = open(dir.path());
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    observer.execute_batch("create table body_updates(value integer); insert into body_updates values(0);
        create trigger count_body_updates after update of body on agent_messages begin update body_updates set value=value+1; end;").unwrap();
    let store = crate::agent::repository::SqliteConversationStore::<AgentEvent>::new(
        database.clone(),
        "stream".into(),
    )
    .unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let lease = WriterLease {
        conversation_id: id,
        writer_id: WriterId::from_uuid(Uuid::new_v4()),
    };
    store.reserve_writer(lease).unwrap();
    store.create(lease).await.unwrap();
    store.persist(lease).await.unwrap();
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let mut events = vec![event(
        id,
        1,
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::AssistantMessage,
        },
    )];
    events.extend((0..128).map(|i| {
        event(
            id,
            i + 2,
            EventPayload::TextDelta {
                turn_id: turn,
                item_id: item,
                text: "你🙂\0".into(),
            },
        )
    }));
    for (i, value) in events.iter().enumerate() {
        store.append(lease, vec![value.clone()]).await.unwrap();
        assert_eq!(store.persist(lease).await.unwrap(), i as u64 + 1);
        assert_eq!(store.flush(lease).await.unwrap(), i as u64 + 1);
    }
    assert_eq!(
        observer
            .query_row("select durable_count from agent_conversations", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    let page = store
        .history(HistoryQuery::new(id, 0, 500).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items, events);
    assert_eq!(
        observer
            .query_row("select body from agent_messages", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "你🙂\0".repeat(128)
    );
    assert_eq!(
        observer
            .query_row("select value from body_updates", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    store.shutdown(lease).await.unwrap();
}

#[test]
fn pending_suffix_reopens_and_pair_loss_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let database = open(dir.path());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let first = command(id, 0, vec![], Some("restart".into()));
    database.write(first, WriteOptions::default()).unwrap();
    drop(database); // No materialization or shutdown: recover the accepted suffix.
    let database = open(dir.path());
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_conversations", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
    drop(database);
    let sql = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    assert!(matches!(
        Database::open(sql, &dir.path().join("missing-log")),
        Err(DataError::Pair(_))
    ));
}

#[test]
fn failed_projection_is_atomic_blocks_admission_and_replays_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let database = open(dir.path());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    database
        .write(
            command(id, 0, vec![], Some("failure".into())),
            WriteOptions::default(),
        )
        .unwrap();
    database.materialize().unwrap();
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    observer.execute_batch("create trigger inject_projection_failure before insert on agent_journal_records begin select raise(abort,'synthetic failure'); end;").unwrap();
    let value = event(
        id,
        1,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    );
    database
        .write(command(id, 0, vec![value], None), WriteOptions::default())
        .unwrap();
    assert!(database.materialize().is_err());
    assert_eq!(
        observer
            .query_row("select durable_count from agent_conversations", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row("select count(*) from data_operation_receipts", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert!(
        database
            .write(
                command(id, 0, vec![], Some("blocked".into())),
                WriteOptions::default()
            )
            .is_err()
    );
    drop(database);
    observer
        .execute_batch("drop trigger inject_projection_failure")
        .unwrap();
    let database = open(dir.path());
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row("select durable_count from agent_conversations", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        1
    );
}

#[test]
fn persistence_diagnostics_keep_stable_code_and_public_operation_context() {
    let diagnostic = "DATA_RUNTIME_ADMISSION: position=19, conversation_id=fixture, batch_id=operation: disk full";
    let error = crate::agent::session_commands::SessionCommandError::from(
        crate::agent::service::AgentServiceError::Storage(
            research_agent_core::conversation::StoreError::Persistence {
                diagnostic: diagnostic.into(),
            },
        ),
    );
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({
            "code":"data_persistence_failed","message":diagnostic,
        })
    );
}
