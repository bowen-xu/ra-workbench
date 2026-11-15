use super::*;

#[test]
fn template_upgrade_preserves_confirmations_checksums_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("templates.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 151);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Paper','now','now');
      insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
      values('run','paper_overview','needs_review','fixture-generation','v1','v1','v1',1,'preserve',lower(hex(zeroblob(32))),0,1000,1000,1000,'now','now');
      insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) values('bundle','run',lower(hex(zeroblob(32))),'v1','valid','now');
      insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,current_revision,review_state,validation_status,original_payload_hash,created_at,updated_at)
      values('candidate','run','bundle','paper_overview_unit_v1',1,1,'unreviewed','valid',lower(hex(zeroblob(32))),'now','now');
      insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,change_reason,created_at)
      values('revision','candidate',1,'{\"kind\":\"paper_overview_unit_v1\",\"fields\":[{\"field\":\"research_question\"}]}',lower(hex(zeroblob(32))),'ai','run','generated','now');
      insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('unit','paper','research_question','Question','Preserved text','question','now','now');
      insert into ai_overview_confirmation_batches(id,candidate_id,candidate_revision,scope_kind,paper_id,previous_candidate_review_state,idempotency_key,created_at)
      values('batch','candidate',1,'field','paper','unreviewed','preserve-confirm','now');
      insert into ai_overview_field_confirmations(id,batch_id,candidate_id,candidate_revision,field_kind,field_payload_hash,decision_kind,target_reading_unit_id,target_applied_revision,previous_snapshot_json,created_at)
      values('confirmation','batch','candidate',1,'research_question',lower(hex(zeroblob(32))),'ai_unchanged','unit',1,'{\"sentinel\":\"unchanged bytes\"}','now');
      insert into ai_overview_confirmation_undos(id,batch_id,idempotency_key,created_at) values('undo','batch','preserve-undo','now');").unwrap();
    let checksums = |db: &Connection| {
        db.prepare(
            "select version,sha256 from migration_checksums where version<=151 order by version",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before = checksums(&db);
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(db.query_row("select previous_snapshot_json from ai_overview_field_confirmations where id='confirmation'",[],|row|row.get::<_,String>(0)).unwrap(),"{\"sentinel\":\"unchanged bytes\"}");
    assert_eq!(
        db.query_row(
            "select body from structured_reading_units where id='unit'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "Preserved text"
    );
    assert_eq!(
        db.query_row(
            "select batch_id from ai_overview_confirmation_undos where id='undo'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "batch"
    );
    assert!(db.execute("update ai_overview_field_confirmations set field_kind='foreign' where id='confirmation'",[]).is_err());
    assert!(
        db.execute(
            "delete from ai_overview_field_confirmations where id='confirmation'",
            []
        )
        .is_err()
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}
