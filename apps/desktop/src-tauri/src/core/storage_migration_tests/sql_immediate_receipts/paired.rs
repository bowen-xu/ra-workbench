use super::{latest, migrate_through};
use crate::{
    agent::repository::journal::{self as journal_storage},
    data_interface::{Database, journal::JournalCommand},
    storage::{AppStore, AppStorePaths, AppStorePorts, StorageError},
};
use research_agent_core::{
    conversation::{ConversationId, HistoryQuery},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::{RaftLog, WriteMode, WriteOptions};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

struct Deployed {
    root: tempfile::TempDir,
    id: ConversationId,
    batches: [Uuid; 2],
    events: Vec<AgentEvent>,
    epoch: Uuid,
    version: i64,
}

impl Deployed {
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
        factory(self.root.path())
    }
}

fn factory(root: &Path) -> Result<AppStore, StorageError> {
    let events = crate::runtime_events::RuntimeEvents::new(|_, _| Ok(()));
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: root.to_owned(),
            agent_configuration_directory: root.join("configuration"),
        },
        AppStorePorts {
            native_note_trace: None,
            writing_context_source: None,
            writing_resource_source: Some(crate::agent::writing_resources::test_sources::source()),
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
                crate::agent::text_tools::editor_barrier::EditorBarrier::for_events(events.clone()),
            ),
            writing_delivery_tools: Arc::new(
                crate::agent::writing_delivery_tool::DeliveryTools::for_events(events.clone()),
            ),
            events,
        },
    )
}

fn deployed(version: i64) -> Deployed {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("research-assistant.sqlite3")).unwrap();
    migrate_through(&mut sql, version);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    let database = Database::open(sql, &root.path().join("data-log")).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let batches = [Uuid::new_v4(), Uuid::new_v4()];
    let mut epoch = Uuid::nil();
    let mut events = Vec::new();
    for (index, batch) in batches.iter().enumerate() {
        let seq = index as u64 + 1;
        let event = AgentEvent::new(
            id,
            seq,
            seq,
            Uuid::new_v4(),
            EventPayload::ItemAccepted {
                turn_id: Uuid::new_v4(),
                item_id: Uuid::new_v4(),
                kind: ItemKind::AssistantMessage,
                text: format!("deployed {version} 合成🙂\0 {index}"),
                phase: None,
                source: None,
            },
        )
        .unwrap();
        let command = JournalCommand::new(
            id,
            *batch,
            seq - 1,
            vec![event.clone()],
            (index == 0).then(|| "paired identity".to_owned()),
            1_790_000_000_000 + index as u64,
        )
        .unwrap();
        let receipt = database
            .write(
                command,
                WriteOptions {
                    mode: if index == 0 {
                        WriteMode::Immediate
                    } else {
                        WriteMode::Automatic
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(receipt.position, seq);
        epoch = receipt.epoch;
        events.push(event);
    }
    // No shutdown/materialize: preserve a real accepted suffix on the old schema.
    drop(database);
    let fixture = Deployed {
        root,
        id,
        batches,
        events,
        epoch,
        version,
    };
    assert_eq!(
        fixture
            .sql()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    fixture
}

fn history(db: &mut Connection, id: ConversationId) -> Vec<AgentEvent> {
    let tx = db.transaction().unwrap();
    let items =
        journal_storage::read_history_in::<AgentEvent>(&tx, HistoryQuery::new(id, 0, 500).unwrap())
            .unwrap()
            .items;
    tx.commit().unwrap();
    items
}

fn indexed(db: &Connection, fixture: &Deployed) {
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    for batch in fixture.batches {
        let operation:String=db.query_row("select operation_id from data_journal_operation_ids where conversation_id=?1 and batch_id=?2",
            [fixture.id.as_uuid().to_string(),batch.to_string()],|row|row.get(0)).unwrap();
        assert_eq!(
            operation,
            Uuid::new_v5(fixture.id.as_uuid(), batch.as_bytes()).to_string()
        );
    }
    assert_eq!(
        db.query_row("select count(*) from data_operation_receipts", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "select count(*) from data_sql_operation_receipts",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        )
        .unwrap(),
        (fixture.epoch.to_string(), 2)
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
}

fn backup(fixture: &Deployed) -> PathBuf {
    let marker = format!(
        "v{}-to-v{}",
        fixture.version,
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
    std::fs::read_dir(fixture.root.path().join("backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(&marker)
        })
        .unwrap()
}

fn assert_backup_restores(fixture: &Deployed, path: &Path) {
    let sql = Connection::open(path).unwrap();
    assert_eq!(latest(&sql), fixture.version);
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let log_path = path.with_extension("raft");
    let locked = Database::lock_existing_pair(&sql, &log_path)
        .unwrap()
        .unwrap();
    assert_eq!(locked.head().unwrap().accepted_position, 2);
    let (mut sql, locked) = Database::prepare_schema_migration(sql, &log_path, locked).unwrap();
    assert_eq!(latest(&sql), fixture.version);
    assert_eq!(history(&mut sql, fixture.id), fixture.events);
    crate::storage::migrate(&mut sql).unwrap();
    let restored = Database::open_with_log(sql, &log_path, Some(locked)).unwrap();
    {
        let mut sql = restored.lock().unwrap();
        assert_eq!(
            latest(&sql),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        indexed(&sql, fixture);
        assert_eq!(history(&mut sql, fixture.id), fixture.events);
    }
    restored.shutdown().unwrap();
}

#[test]
fn real_207_and_210_pending_pairs_drain_before_211_and_backup_reopens_with_original_epoch() {
    for version in [207, 210] {
        let fixture = deployed(version);
        let store = fixture.factory().unwrap();
        let database = store.shared_agent_connection();
        {
            let mut sql = database.lock().unwrap();
            assert_eq!(
                latest(&sql),
                crate::storage_migrations::MIGRATIONS.last().unwrap().0
            );
            indexed(&sql, &fixture);
            assert_eq!(history(&mut sql, fixture.id), fixture.events);
        }
        assert!(RaftLog::open(fixture.log_path(), Some(fixture.epoch)).is_err());
        assert_backup_restores(&fixture, &backup(&fixture));
        database.shutdown().unwrap();
        drop(database);
        drop(store);
        let store = fixture.factory().unwrap();
        let database = store.shared_agent_connection();
        {
            let mut sql = database.lock().unwrap();
            indexed(&sql, &fixture);
            assert_eq!(history(&mut sql, fixture.id), fixture.events);
        }
        database.shutdown().unwrap();
    }
}

#[test]
fn failed_real_211_rolls_back_only_its_transaction_after_old_pair_drain_then_reopens() {
    let fixture = deployed(207);
    fixture.sql().execute_batch("create trigger fail_211 before insert on schema_migrations when new.version=211 begin select raise(abort,'controlled 211 failure'); end").unwrap();
    assert!(fixture.factory().is_err());
    let mut sql = fixture.sql();
    assert_eq!(latest(&sql), 210);
    assert_eq!(history(&mut sql, fixture.id), fixture.events);
    assert_eq!(
        sql.query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        )
        .unwrap(),
        (fixture.epoch.to_string(), 2)
    );
    assert_eq!(sql.query_row("select count(*) from sqlite_schema where name in ('data_sql_operation_receipts','data_journal_operation_ids')",[],|row|row.get::<_,u64>(0)).unwrap(),0);
    assert_eq!(
        sql.query_row(
            "select count(*) from migration_checksums where version=211",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let log = RaftLog::open(fixture.log_path(), Some(fixture.epoch)).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 2);
    drop(log);
    let backup = backup(&fixture);
    let old = Connection::open(&backup).unwrap();
    assert_eq!(latest(&old), 207);
    drop(old);
    // The controlled registry trigger is also part of the real old-pair backup;
    // remove that test-only failure before restoring its production pipeline.
    sql.execute_batch("drop trigger fail_211").unwrap();
    drop(sql);
    let old = Connection::open(&backup).unwrap();
    old.execute_batch("drop trigger fail_211").unwrap();
    drop(old);
    assert_backup_restores(&fixture, &backup);
    let store = fixture.factory().unwrap();
    let database = store.shared_agent_connection();
    {
        let mut sql = database.lock().unwrap();
        assert_eq!(
            latest(&sql),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        indexed(&sql, &fixture);
        assert_eq!(history(&mut sql, fixture.id), fixture.events);
    }
    database.shutdown().unwrap();
}
