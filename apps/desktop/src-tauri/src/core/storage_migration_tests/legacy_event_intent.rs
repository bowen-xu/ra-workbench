//! Deployed claims must not acquire fabricated evidence during the upgrade.
use super::*;

#[test]
fn deployed_216_pending_claim_survives_failed_upgrade_and_reopen_without_intent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy-intent.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 216);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch(
        "insert into bibliographic_items(id,item_type,created_at,updated_at) values('paper','document','now','now');
         insert into agent_conversations(id,title) values('pending','Pending');
         insert into agent_paper_reading_history_migrations(paper_id,legacy_session_id,conversation_id,expected_event_count,status)
         values('paper','legacy','pending',12,'pending');",
    ).unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=216",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_217 before insert on schema_migrations when new.version=217 begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    let schema: (u64, u64) = db.query_row(
        "select (select count(*) from sqlite_schema where name='agent_legacy_event_intents'),
                (select count(*) from pragma_table_info('agent_paper_reading_history_migrations') where name='event_intent_required')",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(schema, (0, 0));
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_217;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (i64, u64, u64, String, u64) = db.query_row(
        "select (select event_intent_required from agent_paper_reading_history_migrations where conversation_id='pending'),
                (select count(*) from agent_legacy_event_intents),
                (select count(*) from agent_legacy_import_recovery_guards where conversation_id='pending'),
                (select sha256 from migration_checksums where version=216),
                (select count(*) from pragma_foreign_key_check)",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    assert_eq!(state, (0, 0, 1, checksum, 0));
}
