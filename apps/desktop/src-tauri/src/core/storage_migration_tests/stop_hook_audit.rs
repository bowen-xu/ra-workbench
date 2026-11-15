use super::*;

#[test]
fn deployed_222_upgrade_rolls_back_and_reopens_with_existing_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 222);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=222",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_223 before insert on schema_migrations when new.version=223 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='agent_stop_hook_admissions'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_223;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state:(String,u64,String,u64)=db.query_row("select (select title from agent_conversations where id='legacy'),(select count(*) from agent_stop_hook_admissions),(select sha256 from migration_checksums where version=222),(select count(*) from pragma_foreign_key_check)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
}
