use super::*;

#[test]
fn deployed_219_upgrade_and_reopen_preserve_original_database() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 219);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=219",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_220 before insert on schema_migrations when new.version=220 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='agent_steering_handoffs'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_220;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state:(String,u64,String,u64)=db.query_row("select (select title from agent_conversations where id='legacy'),(select count(*) from agent_steering_handoffs),(select sha256 from migration_checksums where version=219),(select count(*) from pragma_foreign_key_check)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
}
