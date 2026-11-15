use super::*;

#[test]
fn paper_generation_upgrade_preserves_bindings_and_checksums_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 184);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let before = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Paper','now','now');
        insert into agent_conversations(id,title,archived) values('old','Portrait',0),('current','Reading',1);
        insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
        values('portrait','paper_profile_classification','needs_review','fixture-generation','fixture-v1','v1','v1',1,lower(hex(zeroblob(32))),1000,1000,1000,'before','before');
        insert into ai_scope_items(id,run_id,ordinal,item_kind,object_id,parent_object_id,snapshot_hash,created_at)
        values('scope','portrait',0,'attachment','pdf','paper',lower(hex(zeroblob(32))),'before');
        insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind,created_at)
        values('portrait','old','request','paper_profile_classification','before');
        insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at)
        values('source','paper',lower(hex(zeroblob(32))),1,'now');
        insert into paper_analysis_workflows(id,paper_id,context_snapshot_id,status,provider_id,model_id,prompt_set_version,created_at,updated_at)
        values('reading','paper','source','overview_ready','fixture-generation','fixture-v1','v1','now','now');
        insert into agent_paper_analysis_bindings(workflow_id,conversation_id,paper_id,context_snapshot_id,created_at)
        values('reading','current','paper','source','now');").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row(
            "select conversation_id from paper_generation_conversations where paper_id='paper'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "current"
    );
    db.execute_batch("insert into paper_analysis_workflows(id,paper_id,context_snapshot_id,status,provider_id,model_id,prompt_set_version,created_at,updated_at)
        values('reading-again','paper','source','overview_ready','fixture-generation','fixture-v1','v1','now','now');
        insert into agent_paper_analysis_bindings(workflow_id,conversation_id,paper_id,context_snapshot_id,created_at)
        values('reading-again','current','paper','source','now');").unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    for (version, sha) in before {
        assert_eq!(
            db.query_row(
                "select sha256 from migration_checksums where version=?1",
                [version],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            sha
        );
    }
    assert_eq!(
        db.query_row(
            "select count(*) from agent_paper_analysis_bindings where conversation_id='current'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(
        db.query_row(
            "select archived from agent_conversations where id='current'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    // Migration retains the archived pointer; admission, not upgrade, replaces it.
    assert_eq!(
        db.query_row(
            "select conversation_id from paper_generation_conversations where paper_id='paper'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "current"
    );
    assert!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}
