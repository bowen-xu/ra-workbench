use super::*;

#[test]
fn deployed_220_pending_hook_upgrades_and_reopens_as_queue() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 220);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into agent_conversations(id,title) values('conversation','Existing');
        insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json)
        values('one','conversation','turn',1,null,1,'{}'), ('two','conversation','turn',1,'one',1,'{}');
        insert into agent_active_context_windows(conversation_id,window_id) values('conversation','one');
        insert into agent_compact_session_start(conversation_id,window_id) values('conversation','one');").unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=220",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("create trigger reject_221 before insert on schema_migrations when new.version=221 begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(db.query_row("select window_id from agent_compact_session_start where conversation_id='conversation'", [], |row| row.get::<_, String>(0)).unwrap(), "one");
    assert_eq!(db.query_row("select count(*) from pragma_table_info('agent_compact_session_start') where name='ordinal'", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
    db.execute_batch("drop trigger reject_221;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into agent_compact_session_start(conversation_id,window_id) values('conversation','two')", []).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let rows: Vec<String> = db.prepare("select window_id from agent_compact_session_start where conversation_id='conversation' order by ordinal").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(rows, ["one", "two"]);
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=220",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        checksum
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn deployed_221_queue_and_trigger_upgrade_reopen_without_losing_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owner-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 221);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into agent_conversations(id,title) values('conversation','Existing');
        insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json)
        values('one','conversation','turn',1,null,1,'{}'), ('two','conversation','turn',1,'one',1,'{}');
        insert into agent_compact_session_start(conversation_id,window_id) values('conversation','one'), ('conversation','two');").unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=221",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let rows: Vec<String> = db
        .prepare("select window_id from agent_compact_session_start order by ordinal")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, ["one", "two"]);
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=221",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        checksum
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
}
