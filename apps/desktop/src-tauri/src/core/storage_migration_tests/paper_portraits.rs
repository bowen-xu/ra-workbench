use super::*;

#[test]
fn portrait_upgrade_preserves_existing_candidates_checksums_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("portrait-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 149);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into ai_runs
      (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
      values('old-run','selection_explanation','needs_review','fixture-generation','fixture-v1','v1','v1',1,'preserve',lower(hex(zeroblob(32))),0,1000,1000,1000,'now','now');
      insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at)
      values('old-bundle','old-run',lower(hex(zeroblob(32))),'v1','valid','now');
      insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,current_revision,review_state,validation_status,original_payload_hash,created_at,updated_at)
      values('old-candidate','old-run','old-bundle','selection_explanation_v1',1,1,'unreviewed','valid',lower(hex(zeroblob(32))),'now','now');
      insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,change_reason,created_at)
      values('old-revision','old-candidate',1,'{\"sentinel\":\"unchanged bytes\"}',lower(hex(zeroblob(32))),'ai','old-run','generated','now');").unwrap();
    let checksums = |db: &Connection| -> Vec<(i64, String)> {
        db.prepare(
            "select version,sha256 from migration_checksums where version<=149 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    db.execute_batch("insert into ai_review_decisions(id,candidate_id,candidate_revision,action,rationale,idempotency_key,created_at)
      values('old-decision','old-candidate',1,'defer','preserve review','old-review-key','now');").unwrap();
    let before = checksums(&db);
    db.execute_batch("pragma foreign_keys=off;").unwrap();
    let transaction = db.transaction().unwrap();
    transaction
        .execute_batch(include_str!("../../../migrations/0150_paper_portraits.sql"))
        .unwrap();
    transaction
        .execute(
            "insert into schema_migrations(version,applied_at) values(150,'now')",
            [],
        )
        .unwrap();
    transaction.commit().unwrap();
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let portrait_migration_hash: String = db
        .query_row(
            "select sha256 from migration_checksums where version=150",
            [],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=150",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        portrait_migration_hash
    );
    assert_eq!(
        db.query_row(
            "select rationale from ai_review_decisions where id='old-decision'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "preserve review"
    );
    assert!(
        db.execute(
            "update ai_review_decisions set rationale='overwrite' where id='old-decision'",
            []
        )
        .is_err()
    );
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(
        db.query_row(
            "select payload_json from ai_candidate_revisions where id='old-revision'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "{\"sentinel\":\"unchanged bytes\"}"
    );
    assert_eq!(
        db.query_row(
            "select user_goal from ai_runs where id='old-run'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "preserve"
    );
    db.execute(
        "update ai_runs set task_kind='paper_profile_classification' where id='old-run'",
        [],
    )
    .unwrap();
    db.execute(
        "update ai_candidates set candidate_kind='paper_portrait_v1' where id='old-candidate'",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "update ai_candidates set candidate_kind='unknown' where id='old-candidate'",
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

#[test]
fn portrait_catalog_goal_upgrade_preserves_deployed_152_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("portrait-catalog-upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 152);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into ai_runs
      (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
      values('old-run','selection_explanation','needs_review','fixture-generation','fixture-v1','v1','v1',1,'preserve',lower(hex(zeroblob(32))),0,1000,1000,1000,'now','now');
      insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at)
      values('old-bundle','old-run',lower(hex(zeroblob(32))),'v1','valid','now');
      insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,current_revision,review_state,validation_status,original_payload_hash,created_at,updated_at)
      values('old-candidate','old-run','old-bundle','selection_explanation_v1',1,1,'unreviewed','valid',lower(hex(zeroblob(32))),'now','now');
      insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,change_reason,created_at)
      values('old-revision','old-candidate',1,'{\"sentinel\":\"unchanged bytes\"}',lower(hex(zeroblob(32))),'ai','old-run','generated','now');").unwrap();

    let checksums = |db: &Connection| -> Vec<(i64, String)> {
        db.prepare(
            "select version,sha256 from migration_checksums where version<=152 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before = checksums(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    db.execute_batch("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
      select 'portrait-run','paper_profile_classification',status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at from ai_runs where id='old-run';").unwrap();
    let goal = "分类定义。".repeat(600);
    db.execute(
        "update ai_runs set user_goal=?1 where id='portrait-run'",
        [&goal],
    )
    .unwrap();
    assert!(
        db.execute(
            "update ai_runs set user_goal=?1 where id='old-run'",
            [&goal]
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update ai_runs set user_goal=?1 where id='portrait-run'",
            ["x".repeat(64_001)]
        )
        .is_err()
    );
    drop(db);
    let mut db = Connection::open(path).unwrap();
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(
        db.query_row(
            "select user_goal from ai_runs where id='portrait-run'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        goal
    );
    assert_eq!(
        db.query_row(
            "select payload_json from ai_candidate_revisions where id='old-revision'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "{\"sentinel\":\"unchanged bytes\"}"
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
    assert!(
        db.execute("delete from ai_runs where id='old-run'", [])
            .is_err()
    );
}

#[test]
fn review_reset_upgrade_preserves_deployed_153_checksums_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("review-reset.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 153);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let checksums = |db: &Connection| -> Vec<(i64, String)> {
        db.prepare(
            "select version,sha256 from migration_checksums where version<=153 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before = checksums(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(
        db.query_row(
            "select count(*) from ai_overview_field_review_resets",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
