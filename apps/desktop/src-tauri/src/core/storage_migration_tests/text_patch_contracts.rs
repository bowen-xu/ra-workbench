use super::*;
use rusqlite::params;
#[test]
fn upgrade_and_reopen_preserve_applied_bytes_and_legacy_tool_contracts_without_guessing_read_owners()
 {
    for deployed in [156, 157, 158, 161, 162, 163] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("upgrade.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        migrate_through(&mut db, deployed);
        crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
        let before: Vec<(i64, String)> = db
            .prepare("select version,sha256 from migration_checksums order by version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let conversation = uuid::Uuid::new_v4().to_string();
        let request = uuid::Uuid::new_v4().to_string();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'Legacy')",
            [&conversation],
        )
        .unwrap();
        db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')", params![conversation,request,uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string()]).unwrap();
        if deployed >= 157 {
            db.execute("insert into agent_text_reads(conversation_id,path,version,snapshot_json) values(?1,'notes/root/old.md','old','{}')", [&conversation]).unwrap();
        }
        if deployed >= 158 {
            db.execute("insert into agent_text_tool_contracts(conversation_id,request_id,version) values(?1,?2,1)",params![conversation,request]).unwrap();
        }
        crate::storage::migrate(&mut db).unwrap();
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        for (version, hash) in before {
            assert_eq!(
                db.query_row(
                    "select sha256 from migration_checksums where version=?1",
                    [version],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                hash
            );
        }
        assert_eq!(db.query_row("select version from agent_text_tool_contracts where conversation_id=?1 and request_id=?2",params![conversation,request],|row|row.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row("select count(*) from agent_text_read_scopes", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("select count(*) from agent_text_reads", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            i64::from(deployed >= 157)
        );
    }
}

#[test]
fn snapshot_upgrade_preserves_latest_legacy_candidate_without_inventing_missing_generations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("review-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 161);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let conversation = uuid::Uuid::new_v4().to_string();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Synthetic review')",
        [&conversation],
    )
    .unwrap();
    let candidate = serde_json::json!({"path":"source://synthetic","originalPath":"source://synthetic","scopeRequestId":"00000000-0000-4000-8000-000000000001","baseline":null,"candidate":{"pageId":"synthetic","title":"Synthetic","parentId":null,"position":0,"revision":0,"status":"active","document":{"schemaVersion":1,"pageId":"synthetic","blocks":[]}},"imports":[]});
    db.execute("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state) values(?1,'synthetic','old-proposal',7,'null',?2,'pending')", params![conversation, candidate.to_string()]).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let stored: String = db.query_row("select notes_json from agent_note_patch_snapshots where conversation_id=?1 and proposal_id='old-proposal' and generation=7", [&conversation], |r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        serde_json::json!([candidate])
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_note_patch_snapshots where generation<7",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("select persistence_version from agent_note_patch_candidates where proposal_id='old-proposal'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
}
