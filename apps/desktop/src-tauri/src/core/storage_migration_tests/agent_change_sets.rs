use super::*;
use rusqlite::params;

pub(super) fn fixture() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 80);
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');
        insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');
        insert into agent_conversations(id,title) values('c','c');
        insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json)
        values('c','r','t','s',1,'{}');").unwrap();
    for hash in ["a".repeat(64), "b".repeat(64)] {
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'application/octet-stream',0,'now')", [&hash]).unwrap();
        db.execute("update content_object_gc_candidates set unreferenced_since='old',not_before='old' where content_sha256=?1", [&hash]).unwrap();
    }
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('set','c','r','t','s','m','lease',?1,?2,1)", params!["c".repeat(64),"d".repeat(64)]).unwrap();
    db
}
pub(super) fn add_file(db: &Connection, path: &str) {
    db.execute("insert into agent_change_set_files(change_set_id,relative_path,before_sha256,after_sha256,before_executable,after_executable,source_json) values('set',?1,?2,?3,0,0,'{}')", params![path,"a".repeat(64),"b".repeat(64)]).unwrap();
}
fn count(db: &Connection, table: &str) -> i64 {
    // Test-owned static table names only.
    db.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn proposal_sides_retain_objects_until_last_reference_and_deletion_has_grace() {
    let db = fixture();
    add_file(&db, "first.tex");
    add_file(&db, "second.tex");
    assert_eq!(count(&db, "content_object_gc_candidates"), 0);
    let owners: i64 = db
        .query_row(
            "select count(*) from content_object_references where owner_kind='agent_change_set'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owners, 4);
    assert!(db.execute("delete from content_objects", []).is_err());
    db.execute("update agent_change_sets set status='rejected'", [])
        .unwrap();
    assert_eq!(count(&db, "content_object_gc_candidates"), 0);
    db.execute(
        "delete from agent_change_set_files where relative_path='first.tex'",
        [],
    )
    .unwrap();
    assert_eq!(count(&db, "content_object_gc_candidates"), 0);
    db.execute("delete from agent_change_sets", []).unwrap();
    assert_eq!(count(&db, "content_object_gc_candidates"), 2);
    let grace:bool=db.query_row("select min(julianday(not_before)-julianday(unreferenced_since)) >= 30 from content_object_gc_candidates",[],|r|r.get(0)).unwrap();
    assert!(grace);
}
#[test]
fn proposal_rollback_keeps_gc_state_and_frozen_metadata_rejects_updates() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    add_file(&tx, "main.tex");
    tx.rollback().unwrap();
    assert_eq!(count(&db, "content_object_gc_candidates"), 2);
    assert_eq!(count(&db, "agent_change_set_files"), 0);
    add_file(&db, "main.tex");
    assert!(
        db.execute(
            "update agent_change_sets set inventory_hash=?1",
            ["e".repeat(64)]
        )
        .is_err()
    );
    assert!(
        db.execute("update agent_change_set_files set source_json='{}'", [])
            .is_err()
    );
    assert!(db.execute("insert into agent_change_set_files(change_set_id,relative_path) values('set','missing')",[]).is_err());
    assert!(db.execute("insert into agent_change_set_files(change_set_id,relative_path,after_sha256) values('set','invalid',?1)",["b".repeat(64)]).is_err());
    // Removing a conversation also removes its proposals and starts object grace.
    db.execute("delete from agent_conversations where id='c'", [])
        .unwrap();
    assert_eq!(count(&db, "agent_change_sets"), 0);
    assert_eq!(count(&db, "content_object_gc_candidates"), 2);
}
