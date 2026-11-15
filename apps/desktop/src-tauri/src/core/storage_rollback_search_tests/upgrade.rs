use super::*;
use crate::storage::{AppStore, AppStorePaths, AppStorePorts, StorageError};
use research_data_runtime::RaftLog;
use std::{path::PathBuf, sync::Arc};

#[path = "compaction_upgrade.rs"]
mod compaction_upgrade;

struct Deployed {
    root: tempfile::TempDir,
    ledger: Ledger,
    version: i64,
    epoch: Uuid,
    prefix_position: u64,
    accepted_position: u64,
    checksums: Vec<(i64, String)>,
    sql_receipt: Vec<String>,
    stored_prefix: Vec<Vec<String>>,
}

fn latest(sql: &Connection) -> i64 {
    sql.query_row("select max(version) from schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn checksums(sql: &Connection, through: i64) -> Vec<(i64, String)> {
    sql.prepare("select version,sha256 from migration_checksums where version<=?1 order by version")
        .unwrap()
        .query_map([through], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn sql_receipt(sql: &Connection, version: i64) -> Vec<String> {
    if version < 211 {
        return vec![];
    }
    sql.prepare("select json_array(operation_id,epoch,kind,version,hex(fingerprint),sql_fence_position,result_json) from data_sql_operation_receipts order by operation_id").unwrap()
        .query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
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
    fn backup(&self) -> PathBuf {
        let marker = format!(
            "v{}-to-v{}",
            self.version,
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        std::fs::read_dir(self.root.path().join("backups"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.extension().is_some_and(|value| value == "sqlite3")
                    && path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .contains(&marker)
            })
            .unwrap()
    }
}

fn deployed(version: i64) -> Deployed {
    deployed_with(version, |_, _| {})
}

fn deployed_with(version: i64, seed: impl FnOnce(&SharedDatabase, ConversationId)) -> Deployed {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("research-assistant.sqlite3")).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, version);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    let recorded = checksums(&sql, version);
    let database = Database::open(sql, &root.path().join("data-log")).unwrap();
    let ledger = ledger();
    if version >= 211 {
        let command = authorize(&database, ledger.id);
        let receipt = database.authorize_rollback(&command).unwrap();
        assert!(receipt.replayed);
        assert_eq!(receipt.sql_fence.position, 0);
    } else {
        // This is the actual pre-211 authority transaction, not a forged new
        // SQL receipt. Its deployed behavior remains part of the upgrade input.
        let mut sql = database.lock().unwrap();
        let binding = seed_binding(&sql, ledger.id);
        let tx = sql.transaction().unwrap();
        crate::agent::repository::authorize_rollback_in(
            &tx,
            &AuthorizeRollback {
                operation_id: uuid(4),
                conversation_id: ledger.id,
                binding,
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }
    seed(&database, ledger.id);
    let prefix = append(
        &database,
        &ledger.events[..=ledger.first_marker],
        WriteMode::Immediate,
    );
    let prefix_position = prefix.len() as u64;
    let stored_prefix = preserved(&database.lock().unwrap(), ledger.id);
    let receipt = sql_receipt(&database.lock().unwrap(), version);
    let pending = append(
        &database,
        &ledger.events[ledger.second_marker..],
        WriteMode::Automatic,
    );
    let accepted_position = prefix_position + pending.len() as u64;
    // Do not query through Database::lock: that would drain the suffix before
    // the production startup owner can prove its old-schema ordering.
    drop(database);
    let sql = Connection::open(root.path().join("research-assistant.sqlite3")).unwrap();
    let (epoch, position): (String, u64) = sql
        .query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(position, prefix_position);
    assert_eq!(latest(&sql), version);
    assert_eq!(checksums(&sql, version), recorded);
    assert_eq!(sql_receipt(&sql, version), receipt);
    assert_eq!(
        sql.query_row(
            "select count(*) from sqlite_schema where name='agent_rolled_back_turns'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        u64::from(version >= 212)
    );
    if version >= 212 {
        assert_eq!(
            hidden(&sql, ledger.id),
            [30, 40].map(|turn| uuid(turn).to_string())
        );
    }
    Deployed {
        root,
        ledger,
        version,
        epoch: Uuid::parse_str(&epoch).unwrap(),
        prefix_position,
        accepted_position,
        checksums: recorded,
        sql_receipt: receipt,
        stored_prefix,
    }
}

fn assert_upgraded(sql: &mut Connection, fixture: &Deployed) {
    assert_eq!(
        latest(sql),
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
    assert_eq!(checksums(sql, fixture.version), fixture.checksums);
    assert_eq!(sql_receipt(sql, fixture.version), fixture.sql_receipt);
    assert_eq!(
        sql.query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        )
        .unwrap(),
        (fixture.epoch.to_string(), fixture.accepted_position)
    );
    assert_eq!(
        hidden(sql, fixture.ledger.id),
        [10, 20, 30, 40].map(|turn| uuid(turn).to_string())
    );
    assert_preserved(&fixture.stored_prefix, &preserved(sql, fixture.ledger.id));
    assert_history(sql, fixture.ledger.id, &fixture.ledger.events);
    assert_eq!(
        sql.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "select count(*) from migration_checksums where version=212",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

async fn restore_backup(fixture: &Deployed, path: &Path) {
    let mut sql = Connection::open(path).unwrap();
    assert_eq!(latest(&sql), fixture.version);
    assert_eq!(checksums(&sql, fixture.version), fixture.checksums);
    assert_eq!(sql_receipt(&sql, fixture.version), fixture.sql_receipt);
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        fixture.prefix_position
    );
    assert_history(
        &mut sql,
        fixture.ledger.id,
        &fixture.ledger.events[..=fixture.ledger.first_marker],
    );
    let log_path = path.with_extension("raft");
    let locked = Database::lock_existing_pair(&sql, &log_path)
        .unwrap()
        .unwrap();
    assert_eq!(
        locked.head().unwrap().accepted_position,
        fixture.accepted_position
    );
    let (mut sql, locked) = Database::prepare_schema_migration(sql, &log_path, locked).unwrap();
    assert_eq!(latest(&sql), fixture.version);
    assert_history(&mut sql, fixture.ledger.id, &fixture.ledger.events);
    assert!(RaftLog::open(&log_path, Some(fixture.epoch)).is_err());
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open_with_log(sql, &log_path, Some(locked)).unwrap();
    assert_upgraded(&mut database.lock().unwrap(), fixture);
    assert_only_new(&database).await;
    database.shutdown().unwrap();
}

#[tokio::test]
async fn real_210_and_211_rolled_back_pairs_with_pending_suffix_upgrade_and_backup_restore_on_212()
{
    for version in [210, 211] {
        let fixture = deployed(version);
        let store = fixture.factory().unwrap();
        let database = store.shared_agent_connection();
        assert_upgraded(&mut database.lock().unwrap(), &fixture);
        assert!(RaftLog::open(fixture.log_path(), Some(fixture.epoch)).is_err());
        assert_only_new(&database).await;
        restore_backup(&fixture, &fixture.backup()).await;
        database.shutdown().unwrap();
        drop(database);
        drop(store);
        let store = fixture.factory().unwrap();
        let database = store.shared_agent_connection();
        assert_upgraded(&mut database.lock().unwrap(), &fixture);
        assert_only_new(&database).await;
        database.shutdown().unwrap();
    }
}

#[tokio::test]
async fn failed_212_registry_transaction_preserves_drained_211_pair_and_recovers_on_reopen() {
    let fixture = deployed(211);
    fixture.sql().execute_batch("create trigger fail_212 before insert on schema_migrations when new.version=212 begin select raise(abort,'controlled 212 registry failure'); end").unwrap();
    assert!(fixture.factory().is_err());
    let mut sql = fixture.sql();
    assert_eq!(latest(&sql), 211);
    assert_eq!(checksums(&sql, 211), fixture.checksums);
    assert_eq!(sql_receipt(&sql, 211), fixture.sql_receipt);
    assert_eq!(
        sql.query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        )
        .unwrap(),
        (fixture.epoch.to_string(), fixture.accepted_position)
    );
    assert_history(&mut sql, fixture.ledger.id, &fixture.ledger.events);
    assert_preserved(&fixture.stored_prefix, &preserved(&sql, fixture.ledger.id));
    assert_eq!(
        sql.query_row(
            "select count(*) from sqlite_schema where name='agent_rolled_back_turns'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "select count(*) from migration_checksums where version=212",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let log = RaftLog::open(fixture.log_path(), Some(fixture.epoch)).unwrap();
    assert_eq!(
        log.head().unwrap().accepted_position,
        fixture.accepted_position
    );
    drop(log);
    let backup = fixture.backup();
    let old = Connection::open(&backup).unwrap();
    assert_eq!(latest(&old), 211);
    assert_eq!(sql_receipt(&old, 211), fixture.sql_receipt);
    // The injected failure is intentionally captured by the real backup; only
    // remove this test-owned trigger before exercising the recovery pipeline.
    old.execute_batch("drop trigger fail_212").unwrap();
    sql.execute_batch("drop trigger fail_212").unwrap();
    drop(old);
    drop(sql);
    restore_backup(&fixture, &backup).await;
    let store = fixture.factory().unwrap();
    let database = store.shared_agent_connection();
    assert_upgraded(&mut database.lock().unwrap(), &fixture);
    assert_only_new(&database).await;
    database.shutdown().unwrap();
}

#[test]
fn invalid_historical_rollback_aborts_all_212_backfill_without_rewriting_old_facts() {
    let fixture = deployed(211);
    let corrupt = ConversationId::from_uuid(uuid(2));
    let marker = AgentEvent::new(
        corrupt,
        1,
        123,
        uuid(999),
        P::ConversationRolledBack {
            turn_id: uuid(99),
            user_turns: 1,
        },
    )
    .unwrap();
    let encoded = serde_json::to_string(&marker).unwrap();
    {
        let mut sql = fixture.sql();
        let tx = sql.transaction().unwrap();
        tx.execute("insert into agent_conversations(id,title,durable_count) values(?1,'isolated invalid legacy ledger',1)", [corrupt.as_uuid().to_string()]).unwrap();
        // Simulate a historical direct SQL writer: the envelope is valid, but
        // no user turn exists to satisfy this marker. No valid old fact changes.
        tx.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)", params![corrupt.as_uuid().to_string(),encoded]).unwrap();
        tx.commit().unwrap();
    }
    for _ in 0..2 {
        assert!(fixture.factory().is_err());
        let mut sql = fixture.sql();
        assert_eq!(latest(&sql), 211);
        assert_eq!(checksums(&sql, 211), fixture.checksums);
        assert_eq!(sql_receipt(&sql, 211), fixture.sql_receipt);
        assert_history(&mut sql, fixture.ledger.id, &fixture.ledger.events);
        assert_preserved(&fixture.stored_prefix, &preserved(&sql, fixture.ledger.id));
        assert_eq!(sql.query_row("select payload_json from agent_journal_records where conversation_id=?1 and ordinal=0", [corrupt.as_uuid().to_string()], |row| row.get::<_,String>(0)).unwrap(), encoded);
        assert_eq!(
            sql.query_row(
                "select count(*) from sqlite_schema where name='agent_rolled_back_turns'",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from migration_checksums where version=212",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "select epoch,materialized_position from data_runtime_state",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            )
            .unwrap(),
            (fixture.epoch.to_string(), fixture.accepted_position)
        );
        drop(sql);
        let log = RaftLog::open(fixture.log_path(), Some(fixture.epoch)).unwrap();
        assert_eq!(
            log.head().unwrap().accepted_position,
            fixture.accepted_position
        );
    }
}
