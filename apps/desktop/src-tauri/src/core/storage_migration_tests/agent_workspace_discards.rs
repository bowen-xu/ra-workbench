use super::*;

#[test]
fn workspace_discard_migration_freezes_existing_baselines_without_deleting_history() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 88);
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');
        insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');
        insert into agent_conversations(id,title) values('c','c');
        insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values('c','r','t','s',1,'{}');").unwrap();
    let hash = "a".repeat(64);
    let baseline = r#"{"schemaVersion":1,"fixture":"preserve exact legacy metadata"}"#;
    db.execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json) values('c','r','t','s','m','l',?1,?2)", rusqlite::params![hash, baseline]).unwrap();
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('proposal','c','r','t','s','m','l',?1,?1,1)", [&hash]).unwrap();
    // A legacy row lacking its execution identity must not borrow another baseline.
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('missing','c','r','t','s','m','different',?1,?2,1)", rusqlite::params![hash, "b".repeat(64)]).unwrap();
    let sql = MIGRATIONS
        .iter()
        .find(|(version, _)| *version == 89)
        .unwrap()
        .1;
    let tx = db.transaction().unwrap();
    tx.execute_batch(sql).unwrap();
    tx.commit().unwrap();
    let saved: String = db
        .query_row(
            "select baseline_json from agent_change_sets where id='proposal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, baseline);
    let missing: String = db
        .query_row(
            "select baseline_json from agent_change_sets where id='missing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missing, "null");
    let copies: i64 = db
        .query_row("select count(*) from agent_execution_workspaces", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(copies, 1);
    let discards: i64 = db
        .query_row("select count(*) from agent_workspace_discards", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(discards, 0);
    assert!(
        db.execute("update agent_change_sets set baseline_json='{}'", [])
            .is_err()
    );
    let violations: i64 = db
        .query_row("select count(*) from pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}
