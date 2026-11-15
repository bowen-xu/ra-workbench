use super::*;
use crate::{
    data_interface::journal::JournalCommand,
    storage::{AppStore, AppStorePaths, AppStorePorts},
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind, ModelIdentity, TaskKind},
};
use research_data_runtime::{WriteMode, WriteOptions};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

const BODY: &str = "合成🙂\0\n恢复";
struct Fixture {
    directory: tempfile::TempDir,
    item: Uuid,
    epoch: Uuid,
}
impl Fixture {
    fn sql_path(&self) -> std::path::PathBuf {
        self.directory.path().join("research-assistant.sqlite3")
    }
    fn log_path(&self) -> std::path::PathBuf {
        self.directory.path().join("data-log")
    }
    fn connection(&self) -> Connection {
        let connection = Connection::open(self.sql_path()).unwrap();
        connection.busy_timeout(Duration::ZERO).unwrap();
        connection
    }
    fn prepare(&self) -> Result<(Connection, Option<RaftLog>), StorageError> {
        let sql = self.connection();
        let log = Database::lock_existing_pair(&sql, &self.log_path())?;
        prepare(sql, &self.log_path(), log)
    }
    fn assert_pending(&self) {
        assert_eq!(
            self.connection()
                .query_row(
                    "select materialized_position from data_runtime_state",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        let log = RaftLog::open(self.log_path(), Some(self.epoch)).unwrap();
        assert_eq!(log.head().unwrap().accepted_position, 3);
        assert_eq!(log.head().unwrap().compacted_position, 0);
    }
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("research-assistant.sqlite3");
    let log_path = directory.path().join("data-log");
    let mut sql = Connection::open(path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 207);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    sql.execute_batch("pragma journal_mode=wal;").unwrap();
    let database = Database::open(sql, &log_path).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let event = |seq, payload| AgentEvent::new(id, seq, seq, Uuid::from_u128(42), payload).unwrap();
    let write = |expected, events, title, mode| {
        database
            .write(
                JournalCommand::new(
                    id,
                    Uuid::new_v4(),
                    expected,
                    events,
                    title,
                    1_790_000_000_000,
                )
                .unwrap(),
                WriteOptions {
                    mode,
                    ..WriteOptions::default()
                },
            )
            .unwrap()
    };
    write(
        0,
        vec![
            event(
                1,
                EventPayload::TurnStarted {
                    task_kind: TaskKind::Regular,
                    turn_id: turn,
                    context_snapshot_id: Uuid::new_v4(),
                    model: ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "fixture".into(),
                        revision: None,
                    },
                },
            ),
            event(
                2,
                EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: ItemKind::AssistantMessage,
                },
            ),
        ],
        Some("startup fixture".into()),
        WriteMode::Immediate,
    );
    let epoch: String = database
        .lock()
        .unwrap()
        .query_row("select epoch from data_runtime_state", [], |r| r.get(0))
        .unwrap();
    write(
        2,
        vec![event(
            3,
            EventPayload::TextDelta {
                turn_id: turn,
                item_id: item,
                text: "合成🙂\0\n".into(),
            },
        )],
        None,
        WriteMode::Automatic,
    );
    write(
        3,
        vec![
            event(
                4,
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "恢复".into(),
                },
            ),
            event(
                5,
                EventPayload::ItemCompleted {
                    turn_id: turn,
                    item_id: item,
                },
            ),
        ],
        None,
        WriteMode::Automatic,
    );
    drop(database);
    let result = Fixture {
        directory,
        item,
        epoch: Uuid::parse_str(&epoch).unwrap(),
    };
    result.assert_pending();
    result
}

#[test]
fn deployed_207_pending_suffix_is_drained_before_migrate_and_transfers_one_lock() {
    let fixture = fixture();
    fixture.connection().execute("insert into import_jobs(id,source_kind,state,created_at,updated_at) values('startup-fixture','fixture','running','now','now')", []).unwrap();
    let (mut sql, log) = fixture.prepare().unwrap();
    let log = log.unwrap();
    assert_eq!(log.head().unwrap().epoch, fixture.epoch);
    assert_eq!(log.head().unwrap().accepted_position, 3);
    assert_eq!(log.head().unwrap().synced_position, 3);
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    let body: String = sql
        .query_row(
            "select body from agent_messages where item_id=?1",
            [fixture.item.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, BODY);
    assert_eq!(
        sql.query_row(
            "select state from import_jobs where id='startup-fixture'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running"
    );
    assert!(RaftLog::open(fixture.log_path(), Some(fixture.epoch)).is_err());
    // migrate also runs stranded-task recovery with no pending DDL. Its input
    // must already contain the complete accepted prefix, including old slices.
    crate::storage::migrate(&mut sql).unwrap();
    assert_eq!(
        sql.query_row(
            "select state from import_jobs where id='startup-fixture'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "interrupted"
    );
    let database = Database::open_with_log(sql, &fixture.log_path(), Some(log)).unwrap();
    let body: String = database
        .lock()
        .unwrap()
        .query_row(
            "select body from agent_messages where item_id=?1",
            [fixture.item.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, BODY);
    database.shutdown().unwrap();
    drop(database);
    let (sql, log) = fixture.prepare().unwrap();
    assert_eq!(log.unwrap().head().unwrap().epoch, fixture.epoch);
    assert_eq!(
        sql.query_row("select count(*) from data_operation_receipts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn known_deployed_checksum_exception_is_retained_without_rewriting_it() {
    use sha2::{Digest, Sha256};
    let fixture = fixture();
    let source = MIGRATIONS
        .iter()
        .find(|(version, _)| *version == 63)
        .unwrap()
        .1;
    let recorded = format!("{:x}", Sha256::digest(format!("{source}\n").as_bytes()));
    fixture
        .connection()
        .execute(
            "update migration_checksums set sha256=?1 where version=63",
            [&recorded],
        )
        .unwrap();
    let (sql, _log) = fixture.prepare().unwrap();
    let retained: String = sql
        .query_row(
            "select sha256 from migration_checksums where version=63",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, recorded);
}

#[test]
fn checksum_and_registry_failures_reject_before_projection_without_rewriting_records() {
    let fixture = fixture();
    for (statements, code) in [
        (
            "delete from migration_checksums where version=207",
            "CHECKSUM_MISSING",
        ),
        (
            "update migration_checksums set sha256='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' where version=207",
            "CHECKSUM_MISMATCH",
        ),
        (
            "update migration_checksums set sha256=replace(printf('%064d',0),'0','g') where version=207",
            "CHECKSUM_INVALID",
        ),
    ] {
        fixture.connection().execute_batch(statements).unwrap();
        let error = fixture.prepare().err().unwrap().to_string();
        assert!(error.contains(code), "{error}");
        assert!(!error.contains(&"g".repeat(64)));
        fixture.assert_pending();
        let sql = fixture.connection();
        sql.execute("delete from migration_checksums where version=207", [])
            .unwrap();
        let source = MIGRATIONS
            .iter()
            .find(|(version, _)| *version == 207)
            .unwrap()
            .1;
        crate::storage_integrity::record_migration_checksum(&sql, 207, source).unwrap();
    }
    fixture
        .connection()
        .execute_batch(
            "delete from migration_checksums where version=206;
             delete from schema_migrations where version=206;",
        )
        .unwrap();
    assert!(
        fixture
            .prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("INCOMPLETE_PREFIX")
    );
    fixture.assert_pending();
}

#[test]
fn factory_rejects_unknown_future_schema_before_replaying_a_paired_log() {
    let fixture = fixture();
    let future = MIGRATIONS.last().unwrap().0 + 1;
    fixture
        .connection()
        .execute(
            "insert into schema_migrations(version,applied_at) values(?1,'corrupt-fixture-marker')",
            [future],
        )
        .unwrap();
    let events = crate::runtime_events::RuntimeEvents::new(|_, _| Ok(()));
    let ports = AppStorePorts {
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
    };
    let result = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: fixture.directory.path().to_owned(),
            agent_configuration_directory: fixture.directory.path().join("configuration"),
        },
        ports,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("UNKNOWN_VERSION")
    );
    fixture.assert_pending();
}

#[test]
fn projection_failure_preserves_the_pair_for_a_later_reopen() {
    let fixture = fixture();
    let observer = fixture.connection();
    observer.execute_batch("begin immediate;").unwrap();
    assert!(
        fixture
            .prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("apply_batch")
    );
    fixture.assert_pending();
    observer.execute_batch("rollback;").unwrap();
    let (sql, log) = fixture.prepare().unwrap();
    assert_eq!(log.unwrap().head().unwrap().accepted_position, 3);
    assert_eq!(
        sql.query_row(
            "select body from agent_messages where item_id=?1",
            [fixture.item.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        BODY
    );
}

#[test]
fn sqlite_only_legacy_schema_keeps_the_existing_upgrade_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite3");
    let mut sql = Connection::open(path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 62);
    let (mut sql, log) = prepare(sql, &directory.path().join("data-log"), None).unwrap();
    assert!(log.is_none());
    assert_eq!(
        sql.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        62
    );
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open_with_log(sql, &directory.path().join("data-log"), None).unwrap();
    assert!(database.logged());
}

#[test]
fn fresh_migration_registers_deployed_schema_functions_and_preserves_checksums() {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql)
    .unwrap();
    assert_eq!(
        sql.query_row(
            "select ra_writing_note_title_v1('folder/Note.RANOTE')",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "Note"
    );
    let checksums: String = sql.query_row("select json_group_array(json_array(version,sha256)) from (select version,sha256 from migration_checksums order by version)",[],|row|row.get(0)).unwrap();
    crate::storage::migrate(&mut sql)
    .unwrap();
    assert_eq!(sql.query_row("select json_group_array(json_array(version,sha256)) from (select version,sha256 from migration_checksums order by version)",[],|row|row.get::<_,String>(0)).unwrap(),checksums);
}

#[test]
fn paired_prepare_and_migration_share_registration_connection_and_preserve_replay() {
    let fixture = fixture();
    let sql = fixture.connection();
    // A TEMP trigger requires all deployed SQL functions during the real replay.
    // Its observations also prove replay and migration retain this connection.
    sql.execute_batch(
        "create temp table registration_trace(title text,folded text,identity text);
         create temp trigger require_schema_functions after update on main.agent_messages
         begin
             insert into registration_trace values(
                 ra_writing_note_title_v1('folder/Note.RANOTE'),
                 ra_note_fold_v1('ABC'),
                 data_journal_operation_id('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000002')
             );
         end;",
    )
    .unwrap();
    let locked = Database::lock_existing_pair(&sql, &fixture.log_path()).unwrap();
    let (mut sql, log) = prepare(sql, &fixture.log_path(), locked).unwrap();
    let observed: Vec<(String, String, String)> = sql
        .prepare("select title,folded,identity from temp.registration_trace")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!observed.is_empty());
    let identity = Uuid::new_v5(&Uuid::from_u128(1), Uuid::from_u128(2).as_bytes()).to_string();
    assert!(
        observed
            .iter()
            .all(|row| row == &("Note".into(), "abc".into(), identity.clone()))
    );
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    crate::storage::migrate(&mut sql)
    .unwrap();
    assert_eq!(
        sql.query_row("select count(*) from temp.registration_trace", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap(),
        observed.len()
    );
    let database = Database::open_with_log(sql, &fixture.log_path(), log).unwrap();
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row(
                "select ra_writing_note_title_v1('folder/Note.ranote')",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Note"
    );
    database.shutdown().unwrap();
}

#[test]
fn connection_setup_failure_keeps_its_sql_error_and_does_not_replay_the_pair() {
    let fixture = fixture();
    let sql = fixture.connection();
    let locked = Database::lock_existing_pair(&sql, &fixture.log_path()).unwrap();
    sql.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
        if matches!(
            context.action,
            rusqlite::hooks::AuthAction::Pragma {
                pragma_name: "foreign_keys",
                ..
            }
        ) {
            rusqlite::hooks::Authorization::Deny
        } else {
            rusqlite::hooks::Authorization::Allow
        }
    }));
    let result = prepare(sql, &fixture.log_path(), locked);
    assert!(matches!(
        result,
        Err(StorageError::Database(rusqlite::Error::SqliteFailure(error, _)))
            if error.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
    ));
    fixture.assert_pending();
    let (mut sql, log) = fixture.prepare().unwrap();
    crate::storage::migrate(&mut sql)
    .unwrap();
    let database = Database::open_with_log(sql, &fixture.log_path(), log).unwrap();
    database.shutdown().unwrap();
}
