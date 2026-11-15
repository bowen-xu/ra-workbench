use super::*;

#[test]
fn deployed_218_upgrade_and_reopen_preserve_data_without_inventing_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 218);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=218",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_219 before insert on schema_migrations when new.version=219 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='agent_child_cleanup_intents'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_219;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String, u64, String, u64) = db.query_row("select (select title from agent_conversations where id='legacy'), (select count(*) from agent_child_cleanup_intents), (select sha256 from migration_checksums where version=218), (select count(*) from pragma_foreign_key_check)", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
}
