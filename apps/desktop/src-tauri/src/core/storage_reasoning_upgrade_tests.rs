use super::*;

type MessageRow = (i64, String, String, String, String, u64, u64, bool, String);
type SchemaRow = (String, String, String, Option<String>);

struct Deployed {
    root: tempfile::TempDir,
    id: ConversationId,
    old_item: Uuid,
    epoch: Uuid,
    expected: Vec<AgentEvent>,
}
impl Deployed {
    fn sql_path(&self) -> PathBuf {
        self.root.path().join("research-assistant.sqlite3")
    }
    fn log_path(&self) -> PathBuf {
        self.root.path().join("data-log")
    }
    fn sql(&self) -> Connection {
        let sql = Connection::open(self.sql_path()).unwrap();
        sql.execute_batch("pragma foreign_keys=on").unwrap();
        sql
    }
    fn factory(&self) -> AppStore {
        factory(self.root.path())
    }
}
pub(super) fn factory(root: &Path) -> AppStore {
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
    .unwrap()
}
fn sql207(path: &Path) -> Connection {
    let mut sql = Connection::open(path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 207);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    sql.execute_batch("pragma journal_mode=wal").unwrap();
    sql
}
fn deployed() -> Deployed {
    let root = tempfile::tempdir().unwrap();
    let database = Database::open(
        sql207(&root.path().join("research-assistant.sqlite3")),
        &root.path().join("data-log"),
    )
    .unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let old_item = Uuid::new_v4();
    let mut expected = write(
        &database,
        id,
        0,
        vec![
            P::ItemStarted {
                turn_id: turn,
                item_id: old_item,
                kind: ItemKind::AssistantMessage,
            },
            P::TextDelta {
                turn_id: turn,
                item_id: old_item,
                text: "旧🙂\0".into(),
            },
        ],
        Some("deployed 207"),
        WriteMode::Immediate,
    );
    let epoch: String = database
        .lock()
        .unwrap()
        .query_row("select epoch from data_runtime_state", [], |r| r.get(0))
        .unwrap();
    expected.extend(write(
        &database,
        id,
        2,
        vec![
            P::TextDelta {
                turn_id: turn,
                item_id: old_item,
                text: "\n保真".into(),
            },
            P::ItemAccepted {
                turn_id: turn,
                item_id: old_item,
                kind: ItemKind::AssistantMessage,
                text: OLD_BODY.into(),
                phase: None,
                source: None,
            },
        ],
        None,
        WriteMode::Automatic,
    ));
    let user = Uuid::new_v4();
    expected.extend(write(
        &database,
        id,
        4,
        vec![
            P::ItemStarted {
                turn_id: turn,
                item_id: user,
                kind: ItemKind::UserMessage,
            },
            P::TextDelta {
                turn_id: turn,
                item_id: user,
                text: "旧 user\n".into(),
            },
            P::ItemCompleted {
                turn_id: turn,
                item_id: user,
            },
        ],
        None,
        WriteMode::Automatic,
    ));
    drop(database);
    let result = Deployed {
        root,
        id,
        old_item,
        epoch: Uuid::parse_str(&epoch).unwrap(),
        expected,
    };
    assert_eq!(
        result
            .sql()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    let log = RaftLog::open(result.log_path(), Some(result.epoch)).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 3);
    result
}
fn messages(sql: &Connection) -> Vec<MessageRow> {
    sql.prepare("select rowid,conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal,completed,body from agent_messages order by rowid")
        .unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))
        .unwrap().collect::<Result<_,_>>().unwrap()
}
fn max_version(sql: &Connection) -> i64 {
    sql.query_row("select max(version) from schema_migrations", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn schema(sql: &Connection) -> Vec<SchemaRow> {
    sql.prepare("select type,name,tbl_name,sql from sqlite_schema order by type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn backup_path(root: &Path) -> PathBuf {
    fs::read_dir(root.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "sqlite3"))
        .unwrap()
}
fn assert_207_checksum(sql: &Connection) {
    assert_eq!(
        sql.query_row(
            "select sha256 from migration_checksums where version=207",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0cf807e4c7f6463cb687267e2cb48f994527a3c2e160ae7ead5b908ec943da5d"
    );
}

#[test]
fn factory_drains_real_207_suffix_before_208_209_210_and_restores_its_old_pair_backup() {
    let fixture = deployed();
    let store = fixture.factory();
    let database = store.shared_agent_connection();
    let (upgraded_physical, upgraded_receipts, upgraded_messages) = {
        let mut sql = database.lock().unwrap();
        assert_eq!(
            max_version(&sql),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        assert_207_checksum(&sql);
        assert_fk(&sql);
        assert_eq!(history(&mut sql, fixture.id), fixture.expected);
        assert_eq!(
            sql.query_row(
                "select body from agent_messages where item_id=?1",
                [fixture.old_item.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            OLD_BODY
        );
        assert_eq!(
            sql.query_row(
                "select epoch,materialized_position from data_runtime_state",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
            )
            .unwrap(),
            (fixture.epoch.to_string(), 3)
        );
        (physical(&sql, fixture.id), receipts(&sql), messages(&sql))
    };
    assert!(RaftLog::open(fixture.log_path(), Some(fixture.epoch)).is_err());
    let backup = backup_path(fixture.root.path());
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!(
                "v207-to-v{}",
                crate::storage_migrations::MIGRATIONS.last().unwrap().0
            ))
    );
    let old = Connection::open(&backup).unwrap();
    assert_eq!(max_version(&old), 207);
    assert_eq!(
        old.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let backup_log = backup.with_extension("raft");
    let log = Database::lock_existing_pair(&old, &backup_log).unwrap();
    let (mut old, log) = super::super::startup::prepare(old, &backup_log, log).unwrap();
    assert_eq!(max_version(&old), 207);
    assert_eq!(history(&mut old, fixture.id), fixture.expected);
    assert_eq!(physical(&old, fixture.id), upgraded_physical);
    assert_eq!(receipts(&old), upgraded_receipts);
    assert_eq!(messages(&old), upgraded_messages);
    apply_migration(
        &mut old,
        208,
        include_str!("../../migrations/0208_agent_reasoning_summary.sql"),
    )
    .unwrap();
    assert_eq!(max_version(&old), 208);
    assert_eq!(physical(&old, fixture.id), upgraded_physical);
    assert_eq!(receipts(&old), upgraded_receipts);
    apply_migration(
        &mut old,
        209,
        include_str!("../../migrations/0209_agent_reasoning_part_search.sql"),
    )
    .unwrap();
    assert_eq!(max_version(&old), 209);
    assert_eq!(physical(&old, fixture.id), upgraded_physical);
    assert_eq!(receipts(&old), upgraded_receipts);
    migrate(&mut old).unwrap();
    let restored = Database::open_with_log(old, &backup_log, log).unwrap();
    assert_eq!(
        history(&mut restored.lock().unwrap(), fixture.id),
        fixture.expected
    );
    restored.shutdown().unwrap();
    drop(restored);
    let summary = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let mut expected = fixture.expected.clone();
    expected.extend(write(
        &database,
        fixture.id,
        expected.len() as u64,
        vec![
            P::ItemStarted {
                turn_id: turn,
                item_id: summary,
                kind: summary_kind(json!([])),
            },
            delta(turn, summary, "0", "draft"),
            accepted(
                turn,
                summary,
                "新的🙂\0",
                json!([part("0", 0, "新的🙂\0".len())]),
            ),
        ],
        None,
        WriteMode::Automatic,
    ));
    let combined_receipts = receipts(&database.lock().unwrap());
    assert_eq!(&combined_receipts[..3], upgraded_receipts);
    assert_eq!(combined_receipts.len(), 4);
    database.shutdown().unwrap();
    drop(database);
    drop(store);
    let store = fixture.factory();
    let database = store.shared_agent_connection();
    assert_eq!(receipts(&database.lock().unwrap()), combined_receipts);
    assert_eq!(history(&mut database.lock().unwrap(), fixture.id), expected);
    database.shutdown().unwrap();
}

#[test]
fn failed_208_transaction_rolls_back_schema_but_retains_207_drain_and_backup() {
    let fixture = deployed();
    let raw = fixture.sql();
    let locked = Database::lock_existing_pair(&raw, &fixture.log_path()).unwrap();
    let backup = crate::storage_backup::backup_before_pending_migrations_locked(
        &raw,
        &fixture.root.path().join("backups"),
        locked.as_ref(),
    )
    .unwrap()
    .unwrap();
    let (mut sql, locked) =
        super::super::startup::prepare(raw, &fixture.log_path(), locked).unwrap();
    let original_physical = physical(&sql, fixture.id);
    let original_receipts = receipts(&sql);
    let original_messages = messages(&sql);
    sql.execute_batch("create trigger fail_reasoning_migration before insert on schema_migrations when new.version=208 begin select raise(abort,'controlled 208 rollback'); end").unwrap();
    let original_schema = schema(&sql);
    assert!(
        apply_migration(
            &mut sql,
            208,
            include_str!("../../migrations/0208_agent_reasoning_summary.sql")
        )
        .is_err()
    );
    assert_eq!(schema(&sql), original_schema);
    assert_eq!(max_version(&sql), 207);
    assert_207_checksum(&sql);
    assert_fk(&sql);
    assert_eq!(physical(&sql, fixture.id), original_physical);
    assert_eq!(receipts(&sql), original_receipts);
    assert_eq!(messages(&sql), original_messages);
    assert_eq!(history(&mut sql, fixture.id), fixture.expected);
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(
        sql.query_row(
            "select count(*) from migration_checksums where version=208",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let old = Connection::open(&backup).unwrap();
    assert_eq!(max_version(&old), 207);
    assert_eq!(
        old.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let old_log = RaftLog::open(backup.with_extension("raft"), Some(fixture.epoch)).unwrap();
    assert_eq!(old_log.head().unwrap().accepted_position, 3);
    sql.execute_batch("drop trigger fail_reasoning_migration")
        .unwrap();
    drop(sql);
    drop(locked);
    let store = fixture.factory();
    assert_eq!(
        history(
            &mut store.shared_agent_connection().lock().unwrap(),
            fixture.id
        ),
        fixture.expected
    );
    store.shared_agent_connection().shutdown().unwrap();
}

#[test]
fn preparing_zero_upgrades_from_missing_initialized_or_partial_log_and_reopens() {
    for mode in ["missing", "initialized", "partial"] {
        let root = tempfile::tempdir().unwrap();
        let epoch = Uuid::new_v4();
        let sql = sql207(&root.path().join("research-assistant.sqlite3"));
        sql.execute(
            "insert into data_runtime_state values(1,1,?1,0,'preparing')",
            [epoch.to_string()],
        )
        .unwrap();
        drop(sql);
        let log_path = root.path().join("data-log");
        if mode == "initialized" {
            drop(RaftLog::open(&log_path, Some(epoch)).unwrap());
        } else if mode == "partial" {
            fs::create_dir(&log_path).unwrap();
            fs::File::create(log_path.join("LOCK")).unwrap();
        }
        let store = factory(root.path());
        let database = store.shared_agent_connection();
        {
            let sql = database.lock().unwrap();
            assert_eq!(
                max_version(&sql),
                crate::storage_migrations::MIGRATIONS.last().unwrap().0
            );
            assert_fk(&sql);
            assert_207_checksum(&sql);
            assert_eq!(
                sql.query_row(
                    "select epoch,bootstrap_state,materialized_position from data_runtime_state",
                    [],
                    |r| Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u64>(2)?
                    ))
                )
                .unwrap(),
                (epoch.to_string(), "ready".into(), 0)
            );
        }
        let old = Connection::open(backup_path(root.path())).unwrap();
        assert_eq!(max_version(&old), 207);
        assert_eq!(
            old.query_row("select bootstrap_state from data_runtime_state", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "preparing"
        );
        database.shutdown().unwrap();
        drop(database);
        drop(store);
        let store = factory(root.path());
        assert_eq!(
            store
                .shared_agent_connection()
                .lock()
                .unwrap()
                .query_row("select epoch from data_runtime_state", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            epoch.to_string()
        );
        store.shared_agent_connection().shutdown().unwrap();
    }
}
