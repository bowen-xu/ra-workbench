use super::*;

#[test]
fn deleted_workspace_gc_upgrades_229_and_reopens_without_losing_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-229.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 229);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('existing','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=229",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_230 before insert on schema_migrations when new.version=230 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='agent_deleted_workspace_gc'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_230;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into agent_deleted_workspace_gc(operation_id,conversation_id,turn_id,manuscript_id,lease_id) values('o','c','t','m','l')", []).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_deleted_workspace_gc",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select title from agent_conversations where id='existing'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "Preserved"
    );
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=229",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        checksum
    );
}
