use super::*;

fn fixture(db: &mut Connection) {
    migrate_through(db, 144);
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
    db.execute_batch("insert into agent_conversations(id,title) values('conversation','Existing');
        insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json,created_at)
        values('one','conversation','turn',1,null,1,'{ \"version\":1, \"sentinel\":\"old bytes\" }','old-one'),
              ('two','conversation','turn',2,'one',2,'{ \"version\":2, \"sentinel\":\"old bytes\" }','old-two'),
              ('three','conversation','turn',3,'two',3,'{ \"version\":3, \"sentinel\":\"old bytes\" }','old-three');
        insert into agent_active_context_windows(conversation_id,window_id) values('conversation','three');
        insert into agent_compact_session_start(conversation_id,window_id) values('conversation','three');").unwrap();
}

fn snapshot(db: &Connection) -> Vec<String> {
    [
        "select json_array(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json,created_at) from agent_context_checkpoints order by id",
        "select json_array(conversation_id,window_id) from agent_active_context_windows order by conversation_id",
        "select json_array(conversation_id,window_id) from agent_compact_session_start order by conversation_id",
    ].into_iter().flat_map(|sql| {
        db.prepare(sql).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap()
    }).collect()
}

fn checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=144 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn insert_four(db: &Connection) -> rusqlite::Result<usize> {
    db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) values('four','conversation','turn',4,'three',4,'{}')", [])
}

#[test]
fn provider_window_upgrade_preserves_legacy_bytes_pointers_pending_hooks_and_checksums_on_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    fixture(&mut db);
    let before = snapshot(&db);
    let hashes = checksums(&db);
    assert!(insert_four(&db).is_err());
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(snapshot(&db), before);
    assert_eq!(checksums(&db), hashes);
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(snapshot(&db), before);
    assert_eq!(checksums(&db), hashes);
    insert_four(&db).unwrap();
    assert!(
        db.execute(
            "update agent_context_checkpoints set checkpoint_json='{}' where id='three'",
            []
        )
        .is_err()
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn provider_window_upgrade_foreign_key_failure_rolls_back_schema_data_and_migration_record() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db);
    db.execute_batch("pragma foreign_keys=off;
        insert into agent_conversations(id,title) values('orphan','Fault injection');
        insert into agent_compact_session_start(conversation_id,window_id) values('orphan','missing');
        pragma foreign_keys=on;").unwrap();
    let before = snapshot(&db);
    let hashes = checksums(&db);
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(snapshot(&db), before);
    assert_eq!(checksums(&db), hashes);
    assert!(insert_four(&db).is_err());
    let published: i64 = db
        .query_row(
            "select count(*) from schema_migrations where version=145",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(published, 0);
    let foreign_keys: i64 = db
        .query_row("pragma foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    db.execute(
        "delete from agent_compact_session_start where conversation_id='orphan'",
        [],
    )
    .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    insert_four(&db).unwrap();
}
