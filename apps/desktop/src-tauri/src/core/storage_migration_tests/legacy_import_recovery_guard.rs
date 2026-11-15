//! Upgrade from the deployed schema and retain pending-import evidence.
use super::*;

fn seed(db: &Connection) {
    db.execute_batch("insert into bibliographic_items(id,item_type,created_at,updated_at) values('paper','document','now','now');
        insert into agent_conversations(id,title) values('pending','Pending'),('complete','Complete');
        insert into agent_paper_reading_history_migrations(paper_id,legacy_session_id,conversation_id,expected_event_count,status)
        values('paper','p','pending',12,'pending'),('paper','c','complete',8,'complete');").unwrap();
}
fn guards(db: &Connection) -> Vec<String> {
    db.prepare(
        "select conversation_id from agent_legacy_import_recovery_guards order by conversation_id",
    )
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}
fn old_checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=215 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn deployed_215_backfills_pending_guards_and_retains_them_across_paper_deletion_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy-guard.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 215);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    seed(&db);
    let checksums = old_checksums(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(guards(&db), ["pending"]);
    assert_eq!(old_checksums(&db), checksums);
    db.execute("delete from bibliographic_items where id='paper'", [])
        .unwrap();
    assert_eq!(guards(&db), ["pending"]);
    assert_eq!(
        db.query_row(
            "select count(*) from agent_paper_reading_history_migrations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(guards(&db), ["pending"]);
    assert_eq!(old_checksums(&db), checksums);
    db.execute("delete from agent_conversations where id='pending'", [])
        .unwrap();
    assert!(guards(&db).is_empty());
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn new_pending_and_completed_claims_update_guard_in_their_transaction() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 216);
    seed(&db);
    assert_eq!(guards(&db), ["pending"]);
    db.execute("update agent_paper_reading_history_migrations set status='complete' where conversation_id='pending'", []).unwrap();
    assert!(guards(&db).is_empty());
    db.execute("update agent_paper_reading_history_migrations set status='pending' where conversation_id='complete'", []).unwrap();
    assert_eq!(guards(&db), ["complete"]);
    db.execute(
        "delete from agent_paper_reading_history_migrations where conversation_id='complete'",
        [],
    )
    .unwrap();
    assert_eq!(guards(&db), ["complete"]);
}

#[test]
fn failed_216_registration_rolls_back_ddl_and_backfill_then_reopens() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy-guard-failure.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 215);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    seed(&db);
    let checksums = old_checksums(&db);
    db.execute_batch("create trigger reject_216 before insert on schema_migrations when new.version=216 begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='agent_legacy_import_recovery_guards'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(old_checksums(&db), checksums);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_216;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(guards(&db), ["pending"]);
    assert_eq!(old_checksums(&db), checksums);
}
