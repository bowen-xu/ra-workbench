use super::*;
#[path = "canonical_context_tests.rs"]
mod canonical_context_tests;
use research_agent_core::{
    context::compaction_strategy::{
        CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase,
    },
    protocol::{EventPayload, ModelIdentity, TurnOutcome},
};

pub(super) fn fixture(db: &mut Connection) -> ContextCheckpoint {
    crate::storage_migration_tests::migrate_through(db, 124);
    let conversation = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    };
    super::super::create_conversation(db, ConversationId::from_uuid(conversation), "c").unwrap();
    append(
        db,
        conversation,
        0,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: model.clone(),
        },
    );
    ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: conversation,
        turn_id: turn,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model,
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("verified evidence and remaining work".into()),
        retained: vec![],
        canonical_context: None,
    }
}

#[tokio::test]
async fn shared_store_port_uses_the_same_durable_window() {
    use research_agent_core::context::checkpoint_store::{
        CheckpointInstallation, ContextCheckpointStore,
    };
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    let store = super::super::context_window_store::SqliteContextCheckpointStore::new(
        std::sync::Arc::new(std::sync::Mutex::new(db)),
    );
    assert_eq!(
        store.install(saved.clone()).await.unwrap(),
        CheckpointInstallation::Installed
    );
    assert!(
        store
            .read_active(saved.conversation_id)
            .await
            .unwrap()
            .unwrap()
            == saved
    );
}

#[test]
fn production_upgrade_preserves_recorded_migrations_and_original_history() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 123);
    let conversation = Uuid::new_v4();
    super::super::create_conversation(&mut db, ConversationId::from_uuid(conversation), "original")
        .unwrap();
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let before: Vec<(i64, String)> = {
        let mut query = db.prepare("select version,sha256 from migration_checksums where version <= 123 order by version").unwrap();
        query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    crate::storage::migrate(&mut db).unwrap();
    let after: Vec<(i64, String)> = {
        let mut query = db.prepare("select version,sha256 from migration_checksums where version <= 123 order by version").unwrap();
        query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(before, after);
    assert_eq!(before.len(), 123);
    assert_eq!(
        db.query_row(
            "select title from agent_conversations where id=?1",
            [conversation.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "original"
    );
}
pub(super) fn append(
    db: &mut Connection,
    conversation: Uuid,
    watermark: u64,
    payload: EventPayload,
) {
    let id = ConversationId::from_uuid(conversation);
    let event = AgentEvent::new(id, watermark + 1, 1, Uuid::new_v4(), payload).unwrap();
    super::super::append_batch(db, id, Uuid::new_v4(), watermark, &[event]).unwrap();
}

#[test]
fn persists_complete_windows_across_reopen_without_changing_journal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let saved = fixture(&mut db);
    assert_eq!(install(&mut db, &saved).unwrap(), Installation::Installed);
    assert_eq!(
        install(&mut db, &saved).unwrap(),
        Installation::AlreadyInstalled { active: true }
    );
    let mut next = saved.clone();
    next.window_id = Uuid::new_v4();
    next.previous_window_id = Some(saved.window_id);
    next.output = CompactionOutput::TextSummary("second checkpoint".into());
    assert_eq!(install(&mut db, &next).unwrap(), Installation::Installed);
    assert_eq!(
        install(&mut db, &saved).unwrap(),
        Installation::AlreadyInstalled { active: false }
    );
    drop(db);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("pragma foreign_keys = on;").unwrap();
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == next);
    let count: u64 = db
        .query_row("select durable_count from agent_conversations", [], |row| {
            row.get(0)
        })
        .unwrap();
    let journal: u64 = db
        .query_row("select count(*) from agent_items", [], |row| row.get(0))
        .unwrap();
    assert_eq!((count, journal), (1, 1));
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn rejects_stale_history_terminal_tasks_and_reused_identity() {
    let mut db = Connection::open_in_memory().unwrap();
    let mut saved = fixture(&mut db);
    append(
        &mut db,
        saved.conversation_id,
        1,
        EventPayload::TurnFinished {
            turn_id: saved.turn_id,
            outcome: TurnOutcome::Canceled,
        },
    );
    assert_eq!(install(&mut db, &saved), Err(Error::Conflict));
    saved.source_watermark = 2;
    assert_eq!(install(&mut db, &saved), Err(Error::Inactive));
    assert!(read_active(&db, saved.conversation_id).unwrap().is_none());
    let mut db = Connection::open_in_memory().unwrap();
    let mut saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    saved.output = CompactionOutput::TextSummary("changed".into());
    assert_eq!(install(&mut db, &saved), Err(Error::Conflict));
}

#[test]
fn failed_pointer_write_rolls_back_the_candidate_and_keeps_old_window() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    db.execute_batch("create trigger reject_context_window_update before update on agent_active_context_windows begin select raise(abort,'injected failure'); end;").unwrap();
    let mut next = saved.clone();
    next.window_id = Uuid::new_v4();
    next.previous_window_id = Some(saved.window_id);
    assert_eq!(install(&mut db, &next), Err(Error::Storage));
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        db.execute(
            "update agent_context_checkpoints set checkpoint_json='{}'",
            []
        )
        .is_err()
    );
}

#[test]
fn concurrent_installation_admits_only_one_window_for_the_same_parent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let saved = fixture(&mut db);
    drop(db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let mut joins = vec![];
        for _ in 0..2 {
            let path = path.clone();
            let mut candidate = saved.clone();
            candidate.window_id = Uuid::new_v4();
            let barrier = barrier.clone();
            joins.push(scope.spawn(move || {
                let mut db = Connection::open(path).unwrap();
                db.execute_batch("pragma foreign_keys = on;").unwrap();
                db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                barrier.wait();
                install(&mut db, &candidate)
            }));
        }
        joins
            .into_iter()
            .map(|join| join.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Ok(Installation::Installed))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(Error::Conflict))
            .count(),
        1
    );
}

#[test]
fn coverage_watermark_does_not_consume_new_input_or_weaken_cas() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    let item = Uuid::new_v4();
    for (seq, payload) in [
        (
            1,
            EventPayload::ItemStarted {
                turn_id: saved.turn_id,
                item_id: item,
                kind: research_agent_core::protocol::ItemKind::UserMessage,
            },
        ),
        (
            2,
            EventPayload::TextDelta {
                turn_id: saved.turn_id,
                item_id: item,
                text: "fresh input".into(),
            },
        ),
        (
            3,
            EventPayload::ItemCompleted {
                turn_id: saved.turn_id,
                item_id: item,
            },
        ),
    ] {
        append(&mut db, saved.conversation_id, seq, payload);
    }
    assert_eq!(
        install_owned(&mut db, &saved, 3, None),
        Err(Error::Conflict)
    );
    assert_eq!(
        install_owned(&mut db, &saved, 4, None),
        Ok(Installation::Installed)
    );
    let active = read_active(&db, saved.conversation_id).unwrap().unwrap();
    assert_eq!(active.source_watermark, 1);
    let mut newer = active.clone();
    newer.window_id = Uuid::new_v4();
    newer.previous_window_id = Some(active.window_id);
    newer.source_watermark = 4;
    assert_eq!(
        install_owned(&mut db, &newer, 4, None),
        Ok(Installation::Installed)
    );
    let mut backwards = newer.clone();
    backwards.window_id = Uuid::new_v4();
    backwards.previous_window_id = Some(newer.window_id);
    backwards.source_watermark = 1;
    assert_eq!(
        install_owned(&mut db, &backwards, 4, None),
        Err(Error::Conflict)
    );
    assert_eq!(
        read_active(&db, saved.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        newer.window_id
    );

    let journal = read_projection(&db, saved.conversation_id).unwrap();
    assert_eq!(journal.seq(), 4);
    assert_eq!(
        journal.turn(saved.turn_id).unwrap().items[&item].text,
        "fresh input"
    );
}

#[test]
fn cancellation_before_commit_rolls_back_candidate_and_pointer() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    let cancellation = crate::agent::cancellation::AgentCancellation::default();
    cancellation.cancel();
    assert_eq!(
        install_owned(&mut db, &saved, 1, Some(&cancellation)),
        Err(Error::Canceled)
    );
    assert!(read_active(&db, saved.conversation_id).unwrap().is_none());
    let count: u64 = db
        .query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        read_projection(&db, saved.conversation_id).unwrap().seq(),
        1
    );
}

fn seed_responses_binding_125(db: &mut Connection, saved: &ContextCheckpoint) {
    db.execute_batch(include_str!(
        "../../../../migrations/0125_agent_responses_window_bindings.sql"
    ))
    .unwrap();
    db.execute(
        "insert into schema_migrations(version,applied_at) values(125,'now')",
        [],
    )
    .unwrap();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')",
        params![saved.conversation_id.to_string(), Uuid::new_v4().to_string(), saved.turn_id.to_string(), Uuid::new_v4().to_string()]).unwrap();
    db.execute(
        "insert into agent_responses_window_bindings(turn_id,window_id) values(?1,?2)",
        params![saved.turn_id.to_string(), saved.window_id.to_string()],
    )
    .unwrap();
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
}

#[test]
fn empty_window_upgrade_preserves_existing_pointer_binding_and_reopens() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("checkpoint-upgrade.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let saved = fixture(&mut db);
    assert_eq!(install(&mut db, &saved).unwrap(), Installation::Installed);
    seed_responses_binding_125(&mut db, &saved);
    let checksums: Vec<(i64, String)> = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    assert_eq!(
        db.query_row(
            "select window_id from agent_responses_window_bindings where turn_id=?1",
            [saved.turn_id.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        saved.window_id.to_string()
    );
    let after: Vec<(i64, String)> = db
        .prepare(
            "select version,sha256 from migration_checksums where version <= 125 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(checksums, after);
    assert_eq!(after.len(), 125);
    assert_eq!(
        read_projection(&db, saved.conversation_id).unwrap().seq(),
        1
    );
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |row| row.get::<_, u8>(0))
            .unwrap(),
        1
    );
    assert!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert!(
        db.execute(
            "update agent_context_checkpoints set source_watermark=0 where id=?1",
            [saved.window_id.to_string()]
        )
        .is_err()
    );
    // Existing uniqueness and cascade remain, including dependent replay binding.
    db.execute(
        "delete from agent_conversations where id=?1",
        [saved.conversation_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_responses_window_bindings",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn rejected_empty_window_upgrade_rolls_back_parent_table_and_restores_foreign_keys() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    seed_responses_binding_125(&mut db, &saved);
    db.execute_batch("pragma foreign_keys=off").unwrap();
    db.execute(
        "update agent_responses_window_bindings set window_id='missing-window'",
        [],
    )
    .unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert!(read_active(&db, saved.conversation_id).unwrap().unwrap() == saved);
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |row| row.get::<_, u8>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=126",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_master where name='agent_context_checkpoints_0126'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,schema_version,checkpoint_json) values(?1,?2,?3,0,1,'{}')",
        params![Uuid::new_v4().to_string(), saved.conversation_id.to_string(), saved.turn_id.to_string()]).is_err());
}

#[test]
fn reset_window_reopens_canonical_context_without_rewriting_original_events() {
    use research_agent_core::context::checkpoint::{
        CanonicalContext, StoredContent, StoredMessage,
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("reset.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let mut saved = fixture(&mut db);
    crate::storage::migrate(&mut db).unwrap();
    let original: String = db
        .query_row(
            "select json_group_array(payload_json) from agent_items",
            [],
            |row| row.get(0),
        )
        .unwrap();
    saved.schema_version = 2;
    saved.binding.kind = CompactionKind::ResetWindow;
    saved.output = CompactionOutput::ResetWindow;
    saved.canonical_context = Some(CanonicalContext {
        before_retained: 0,
        messages: vec![StoredMessage::User {
            source_item_id: None,
            content: vec![StoredContent::Text {
                text: "frozen host context".into(),
            }],
        }],
    });
    assert_eq!(install(&mut db, &saved).unwrap(), Installation::Installed);
    assert_eq!(
        install(&mut db, &saved).unwrap(),
        Installation::AlreadyInstalled { active: true }
    );
    drop(db);
    let db = Connection::open(&path).unwrap();
    let restored = read_active(&db, saved.conversation_id).unwrap().unwrap();
    assert!(restored == saved);
    assert_eq!(restored.restore_reset_window(&|_| None).unwrap().len(), 1);
    let after: String = db
        .query_row(
            "select json_group_array(payload_json) from agent_items",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, original);
}
