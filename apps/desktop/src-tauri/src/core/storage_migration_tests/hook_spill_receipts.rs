use super::*;

fn fixture(path: &std::path::Path) -> Connection {
    let mut db = Connection::open(path).unwrap();
    migrate_through(&mut db, 146);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into note_pages(id,title,created_at,updated_at) values('retained-page','Retained','now','now');
        insert into reading_note_sources(block_id,page_id,note_type,color,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
        values('retained-source','retained-page','annotation','#f4cf65','missing','Title','missing','source.pdf','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,'[]','Evidence','','',1,'now','now');").unwrap();
    db
}
fn references(db: &Connection) -> Vec<(String, String, String, String)> {
    db.prepare("select content_sha256,owner_kind,owner_id,role from content_object_references order by content_sha256,owner_kind,owner_id,role").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
}
fn hashes(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=146 order by version")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
#[test]
fn hook_spill_receipts_upgrade_retains_references_and_old_checksums_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite");
    let mut db = fixture(&path);
    let before = references(&db);
    let checksums = hashes(&db);
    assert_eq!(before.len(), 1);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(references(&db), before);
    assert_eq!(hashes(&db), checksums);
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(references(&db), before);
    assert_eq!(hashes(&db), checksums);
}
#[test]
fn hook_spill_receipts_migration_failure_restores_previous_view_and_schema() {
    let root = tempfile::tempdir().unwrap();
    let mut db = fixture(&root.path().join("fault.sqlite"));
    let before = references(&db);
    let checksums = hashes(&db);
    db.execute_batch("create trigger reject_147 before insert on schema_migrations when new.version=147 begin select raise(abort,'migration fault'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(references(&db), before);
    assert_eq!(hashes(&db), checksums);
    assert_eq!(db.query_row("select count(*) from sqlite_master where name in ('agent_hook_spill_artifacts','agent_hook_spill_previews','agent_hook_spill_grants')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute_batch("drop trigger reject_147").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(references(&db), before);
}

#[path = "hook_spill_receipts/cleanup_upgrade.rs"]
mod cleanup_upgrade;
