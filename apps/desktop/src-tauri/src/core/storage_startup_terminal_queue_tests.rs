use super::*;
use crate::{
    agent::repository::{create_conversation, input_queue},
    data_interface::journal::JournalCommand,
    storage::{AppStore, AppStorePaths, AppStorePorts},
};
use research_agent_core::{
    conversation::{
        BindingKind, ConversationBinding, ConversationId, HistoryQuery, RenameConversation,
    },
    input_queue::{QueueEdit, QueuedInput},
    protocol::{
        AgentEvent, ConversationProjection, EventPayload, FailureCategory, ModelIdentity, TaskKind,
        TurnOutcome,
    },
    submission::{SubmissionInput, SubmissionRecord},
    user_input::UserInput,
};
use research_data_runtime::{Durability, WriteMode, WriteOptions};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

struct Fixture {
    root: tempfile::TempDir,
    record: SubmissionRecord,
    events: Vec<AgentEvent>,
    queue: Value,
    preserved: Vec<String>,
    checksums: Vec<(i64, String)>,
    epoch: Uuid,
    terminal_batch: Uuid,
    version: i64,
}
impl Fixture {
    fn sql_path(&self) -> PathBuf {
        self.root.path().join("research-assistant.sqlite3")
    }
    fn log_path(&self) -> PathBuf {
        self.root.path().join("data-log")
    }
    fn sql(&self) -> Connection {
        Connection::open(self.sql_path()).unwrap()
    }
    fn factory(&self) -> Result<AppStore, StorageError> {
        let events = crate::runtime_events::RuntimeEvents::new(|_, _| Ok(()));
        AppStore::open_with_paths(
            AppStorePaths {
                data_directory: self.root.path().to_owned(),
                agent_configuration_directory: self.root.path().join("configuration"),
            },
            AppStorePorts {
                native_note_trace: None,
                writing_context_source: None,
                writing_resource_source: Some(
                    crate::agent::writing_resources::test_sources::source(),
                ),
                paper_metadata_source: None,
                retrieval_trace_source: None,
                retrieval_preview_source: None,
                semantic_index_source: None,
                paper_portrait_source: None,
                context_catalog_source: crate::context_catalog_app::source(),
                generation_policy_source: None,
                paper_generation_source: None,
                paper_reading_source: None,
                paper_analysis_source: None,
                candidate_recovery_source: None,
                writing_compile_feedback_source: None,
                #[cfg(target_os = "macos")]
                writing_workspace_compile_source: None,
                #[cfg(unix)]
                writing_workspace_source: None,
                text_editors: Arc::new(
                    crate::agent::text_tools::editor_barrier::EditorBarrier::for_events(
                        events.clone(),
                    ),
                ),
                writing_delivery_tools: Arc::new(
                    crate::agent::writing_delivery_tool::DeliveryTools::for_events(events.clone()),
                ),
                events,
            },
        )
    }
    fn assert_pending(&self) {
        let sql = self.sql();
        assert_eq!(
            sql.query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row(
                "select durable_count from agent_conversations where id=?1",
                [self.record.conversation_id.to_string()],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
            self.events.len() - 1
        );
        assert_eq!(queue(&sql, self.record.conversation_id), self.queue);
        assert_eq!(preserved(&sql), self.preserved);
        assert_eq!(checksums(&sql, self.version), self.checksums);
        drop(sql);
        let log = RaftLog::open(self.log_path(), Some(self.epoch)).unwrap();
        assert_eq!(log.head().unwrap().accepted_position, 2);
        assert_eq!(log.head().unwrap().synced_position, 2);
    }
    fn assert_recovered(&self, sql: &mut Connection) {
        assert_eq!(
            latest(sql),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        assert_eq!(checksums(sql, self.version), self.checksums);
        assert_eq!(preserved(sql), self.preserved);
        assert_eq!(
            sql.query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            2
        );
        let mut expected_queue = self.queue.clone();
        expected_queue["revision"] = json!(self.queue["revision"].as_u64().unwrap() + 1);
        expected_queue["paused"] = json!(true);
        assert_eq!(queue(sql, self.record.conversation_id), expected_queue);
        let history = crate::agent::repository::journal::read_history::<AgentEvent>(
            sql,
            HistoryQuery::new(
                ConversationId::from_uuid(self.record.conversation_id),
                0,
                100,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(history.items, self.events);
        let operation = Uuid::new_v5(&self.record.conversation_id, self.terminal_batch.as_bytes());
        let (epoch, position, result): (String,u64,String) = sql.query_row("select epoch,position,result_json from data_operation_receipts where operation_id=?1", [operation.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            (epoch, position, result),
            (self.epoch.to_string(), 2, self.events.len().to_string())
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from schema_migrations where version=215",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    }
}
fn latest(sql: &Connection) -> i64 {
    sql.query_row("select max(version) from schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn queue(sql: &Connection, conversation: Uuid) -> Value {
    let json: String = sql
        .query_row(
            "select snapshot_json from agent_input_queues where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&json).unwrap()
}
fn checksums(sql: &Connection, through: i64) -> Vec<(i64, String)> {
    sql.prepare("select version,sha256 from migration_checksums where version<=?1 order by version")
        .unwrap()
        .query_map([through], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn preserved(sql: &Connection) -> Vec<String> {
    // These immutable locators and exact SQL proof retain R, its original fence,
    // payload, queue claim revision and receipt even while the suffix is blocked.
    let mut values = Vec::new();
    for query in [
        "select json_array(conversation_id,request_id,turn_id,context_snapshot_id,input_json) from agent_submissions order by request_id",
        "select json_array(input_id,conversation_id,request_id,expected_revision,result_json) from agent_input_queue_previews order by input_id",
        "select json_array(conversation_id,request_id,input_id) from agent_queue_submission_origins order by request_id",
        "select json_array(request_id,conversation_id,command_json,result_json) from agent_input_queue_receipts order by request_id",
        "select json_array(operation_id,epoch,kind,version,hex(fingerprint),sql_fence_position,result_json) from data_sql_operation_receipts order by operation_id",
    ] {
        values.extend(
            sql.prepare(query)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        );
    }
    values
}
fn fixture(version: i64, snapshot_version: u64, outcome: TurnOutcome) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("research-assistant.sqlite3")).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, version);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    let original_checksums = checksums(&sql, version);
    let conversation = Uuid::new_v4();
    create_conversation(&mut sql, ConversationId::from_uuid(conversation), "Before").unwrap();
    let database = Database::open(sql, &root.path().join("data-log")).unwrap();
    let renamed = database
        .rename_conversation(&RenameConversation {
            operation_id: Uuid::new_v4(),
            id: ConversationId::from_uuid(conversation),
            expected_title: "Before".into(),
            title: "After".into(),
        })
        .unwrap();
    assert_eq!(renamed.sql_fence.position, 0);
    let input_id = Uuid::new_v4();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "queued text".into(),
        user_input: vec![UserInput::Text {
            text: "queued text".into(),
            text_elements: vec![],
        }],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    let (record, frozen_queue, saved) = {
        let mut sql = database.lock().unwrap();
        let mut q = input_queue::apply_checked(
            &mut sql,
            conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: input.user_input.clone(),
                origin: Some(ConversationBinding {
                    kind: BindingKind::Manuscript,
                    resource_id: Uuid::new_v4().to_string(),
                }),
            }),
            |_| Ok(()),
        )
        .unwrap();
        if snapshot_version == 4 {
            let tail = Uuid::new_v4();
            q = input_queue::apply_checked(
                &mut sql,
                conversation,
                Uuid::new_v4(),
                q.revision(),
                QueueEdit::Add(QueuedInput {
                    id: tail,
                    text: "held tail".into(),
                    user_input: vec![],
                    origin: None,
                }),
                |_| Ok(()),
            )
            .unwrap();
            q = input_queue::apply_checked(
                &mut sql,
                conversation,
                Uuid::new_v4(),
                q.revision(),
                QueueEdit::PauseInput(tail),
                |_| Ok(()),
            )
            .unwrap();
        }
        let (record, _) = input_queue::prepare_preview_checked(
            &mut sql,
            conversation,
            Uuid::new_v4(),
            q.revision(),
            input_id,
            input,
            |_| Ok(()),
        )
        .unwrap();
        let frozen = queue(&sql, conversation);
        assert_eq!(frozen["version"], snapshot_version);
        let saved = preserved(&sql);
        (record, frozen, saved)
    };
    let id = ConversationId::from_uuid(conversation);
    let event = |seq, payload| AgentEvent::new(id, seq, seq, record.request_id, payload).unwrap();
    let mut events = vec![event(
        1,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        },
    )];
    if outcome == TurnOutcome::Failed {
        events.push(event(
            2,
            EventPayload::TurnFailed {
                turn_id: record.turn_id,
                error: "Synthetic accepted failure".into(),
                category: FailureCategory::Provider,
            },
        ));
    }
    database
        .write(
            JournalCommand::new(
                id,
                Uuid::new_v4(),
                0,
                events.clone(),
                None,
                1_790_000_000_000,
            )
            .unwrap(),
            WriteOptions {
                mode: WriteMode::Immediate,
                ..WriteOptions::default()
            },
        )
        .unwrap();
    let epoch: String = database
        .lock()
        .unwrap()
        .query_row("select epoch from data_runtime_state", [], |row| row.get(0))
        .unwrap();
    let terminal = event(
        events.len() as u64 + 1,
        EventPayload::TurnFinished {
            turn_id: record.turn_id,
            outcome,
        },
    );
    let terminal_batch = Uuid::new_v4();
    database
        .write(
            JournalCommand::new(
                id,
                terminal_batch,
                events.len() as u64,
                vec![terminal.clone()],
                None,
                1_790_000_000_001,
            )
            .unwrap(),
            WriteOptions {
                mode: WriteMode::Automatic,
                durability: Durability::Device,
            },
        )
        .unwrap();
    events.push(terminal);
    let mut projection = ConversationProjection::new(id);
    for event in &events {
        projection.apply(event).unwrap();
    }
    assert_eq!(
        projection.turn(record.turn_id).unwrap().outcome,
        Some(outcome)
    );
    if version < 215 {
        assert!(database.materialize().is_err());
        assert!(database.materialize().is_err()); // The old owner really is blocked.
    }
    drop(database);
    let fixture = Fixture {
        root,
        record,
        events,
        queue: frozen_queue,
        preserved: saved,
        checksums: original_checksums,
        epoch: Uuid::parse_str(&epoch).unwrap(),
        terminal_batch,
        version,
    };
    fixture.assert_pending();
    fixture
}

#[test]
fn deployed_219_paired_log_upgrades_and_reopens_with_pending_history() {
    let f = fixture(219, 4, TurnOutcome::Interrupted);
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
    drop(db);
    drop(store);
    let reopened = f.factory().unwrap();
    let db = reopened.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn deployed_217_pending_log_replays_before_interleaving_version_fence() {
    let f = fixture(217, 4, TurnOutcome::Interrupted);
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    {
        let mut sql = db.lock().unwrap();
        f.assert_recovered(&mut sql);
        let versions = applied_versions(&sql).unwrap();
        assert_eq!(
            versions.last(),
            MIGRATIONS.last().map(|(version, _)| version)
        );
        let previous = MIGRATIONS
            .iter()
            .copied()
            .filter(|(version, _)| *version <= 217)
            .collect::<Vec<_>>();
        assert!(
            validate_known_versions_against(&versions, &previous)
                .unwrap_err()
                .to_string()
                .contains("DATA_RUNTIME_MIGRATION_UNKNOWN_VERSION: version=218")
        );
    }
    db.shutdown().unwrap();
    drop(db);
    drop(store);
    let reopened = f.factory().unwrap();
    let db = reopened.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn failed_interleaving_version_fence_keeps_replayed_suffix_for_retry() {
    let f = fixture(217, 4, TurnOutcome::Interrupted);
    f.sql().execute_batch("create trigger reject_interleaving_version before insert on schema_migrations when new.version=218 begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(f.factory().is_err());
    {
        let mut sql = f.sql();
        assert_eq!(latest(&sql), 217);
        assert_eq!(checksums(&sql, 217), f.checksums);
        assert_eq!(
            sql.query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            2
        );
        let history = crate::agent::repository::journal::read_history::<AgentEvent>(
            &mut sql,
            HistoryQuery::new(ConversationId::from_uuid(f.record.conversation_id), 0, 100).unwrap(),
        )
        .unwrap();
        assert_eq!(history.items, f.events);
        sql.execute_batch("drop trigger reject_interleaving_version;")
            .unwrap();
    }
    let reopened = f.factory().unwrap();
    let db = reopened.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn deployed_216_pending_log_replays_through_217_without_fabricated_intent() {
    let f = fixture(216, 4, TurnOutcome::Interrupted);
    {
        let db = f.sql();
        crate::notes::register_projection_functions(&db).unwrap();
        crate::storage::schema_compat::register_functions(&db).unwrap();
        db.execute("insert into bibliographic_items(id,item_type,created_at,updated_at) values('synthetic-paper','document','now','now')", []).unwrap();
        db.execute("insert into agent_paper_reading_history_migrations(paper_id,legacy_session_id,conversation_id,expected_event_count,status) values('synthetic-paper','legacy',?1,100,'pending')", [f.record.conversation_id.to_string()]).unwrap();
    }
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    {
        let mut sql = db.lock().unwrap();
        f.assert_recovered(&mut sql);
        let evidence: (i64, u64, u64) = sql.query_row(
            "select (select event_intent_required from agent_paper_reading_history_migrations where conversation_id=?1),
                    (select count(*) from agent_legacy_event_intents where conversation_id=?1),
                    (select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1)",
            [f.record.conversation_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(evidence, (0, 0, 1));
    }
    db.shutdown().unwrap();
    drop(db);
    drop(store);
    let reopened = f.factory().unwrap();
    let db = reopened.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn deployed_215_pending_log_replays_before_216_backfill_and_survives_reopen() {
    let f = fixture(215, 4, TurnOutcome::Interrupted);
    {
        let db = f.sql();
        crate::notes::register_projection_functions(&db).unwrap();
        crate::storage::schema_compat::register_functions(&db).unwrap();
        db.execute("insert into bibliographic_items(id,item_type,created_at,updated_at) values('synthetic-paper','document','now','now')", []).unwrap();
        db.execute("insert into agent_paper_reading_history_migrations(paper_id,legacy_session_id,conversation_id,expected_event_count,status) values('synthetic-paper','legacy',?1,100,'pending')", [f.record.conversation_id.to_string()]).unwrap();
    }
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    {
        let mut sql = db.lock().unwrap();
        f.assert_recovered(&mut sql);
        sql.execute(
            "delete from bibliographic_items where id='synthetic-paper'",
            [],
        )
        .unwrap();
        assert_eq!(
            sql.query_row(
                "select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1",
                [f.record.conversation_id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    }
    db.shutdown().unwrap();
    drop(db);
    drop(store);
    let reopened = f.factory().unwrap();
    let db = reopened.shared_agent_connection();
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_legacy_import_recovery_guards where conversation_id=?1",
                [f.record.conversation_id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    db.shutdown().unwrap();
}

#[test]
fn actual_213_pending_terminal_v3_v4_replays_through_registered_215_and_reopens() {
    for snapshot in [3, 4] {
        for outcome in [
            TurnOutcome::Failed,
            TurnOutcome::Canceled,
            TurnOutcome::Interrupted,
        ] {
            let f = fixture(213, snapshot, outcome);
            let store = f.factory().unwrap();
            let db = store.shared_agent_connection();
            f.assert_recovered(&mut db.lock().unwrap());
            assert!(RaftLog::open(f.log_path(), Some(f.epoch)).is_err());
            let backup = std::fs::read_dir(f.root.path().join("backups"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "sqlite3")
                })
                .unwrap();
            let old = Connection::open(&backup).unwrap();
            assert_eq!(latest(&old), 213);
            assert_eq!(queue(&old, f.record.conversation_id), f.queue);
            assert_eq!(preserved(&old), f.preserved);
            assert_eq!(
                old.query_row(
                    "select materialized_position from data_runtime_state",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                1
            );
            drop(old);
            let backup_log = RaftLog::open(backup.with_extension("raft"), Some(f.epoch)).unwrap();
            assert_eq!(backup_log.head().unwrap().accepted_position, 2);
            drop(backup_log);
            db.shutdown().unwrap();
            drop(db);
            drop(store);
            let store = f.factory().unwrap();
            let db = store.shared_agent_connection();
            f.assert_recovered(&mut db.lock().unwrap());
            db.shutdown().unwrap();
        }
    }
}

#[test]
fn failed_215_registration_leaves_old_trigger_and_the_exact_pending_suffix() {
    let f = fixture(213, 4, TurnOutcome::Failed);
    f.sql().execute_batch("create trigger reject_215 before insert on schema_migrations when new.version=215 begin select raise(abort,'synthetic 215 registration failure'); end;").unwrap();
    assert!(f.factory().is_err());
    f.assert_pending();
    let sql = f.sql();
    assert_eq!(latest(&sql), 213);
    assert_eq!(
        sql.query_row(
            "select sql from sqlite_schema where type='trigger' and name=?1",
            [TRIGGER],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        deployed_trigger().unwrap()
    );
    sql.execute_batch("drop trigger reject_215;").unwrap();
    drop(sql);
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn committed_215_with_replay_failure_recovers_on_reopen_without_repeating_ddl() {
    let f = fixture(213, 4, TurnOutcome::Interrupted);
    f.sql().execute_batch("create trigger reject_terminal before insert on agent_journal_records when json_extract(new.payload_json,'$.payload.type')='turnFinished' begin select raise(abort,'synthetic replay failure after 215'); end;").unwrap();
    assert!(f.factory().is_err());
    f.assert_pending();
    let sql = f.sql();
    assert_eq!(latest(&sql), 215);
    sql.execute_batch("drop trigger reject_terminal; create trigger reject_repeated_215 before insert on schema_migrations when new.version=215 begin select raise(abort,'repeated 215'); end;").unwrap();
    drop(sql);
    let store = f.factory().unwrap();
    let db = store.shared_agent_connection();
    f.assert_recovered(&mut db.lock().unwrap());
    db.shutdown().unwrap();
}

#[test]
fn altered_deployed_trigger_fails_before_compatibility_ddl_or_replay() {
    let f = fixture(213, 3, TurnOutcome::Canceled);
    f.sql().execute_batch("drop trigger agent_terminal_queue_pause; create trigger agent_terminal_queue_pause after insert on agent_journal_records begin select 1; end;").unwrap();
    assert!(
        f.factory()
            .err()
            .unwrap()
            .to_string()
            .contains("TERMINAL_TRIGGER_MISMATCH")
    );
    assert_eq!(latest(&f.sql()), 213);
    f.assert_pending();
}

#[test]
fn older_212_v4_pending_terminal_is_not_misrepresented_as_supported_by_the_213_edge() {
    let f = fixture(212, 4, TurnOutcome::Canceled);
    assert!(f.factory().is_err());
    assert_eq!(latest(&f.sql()), 212);
    f.assert_pending();
}
