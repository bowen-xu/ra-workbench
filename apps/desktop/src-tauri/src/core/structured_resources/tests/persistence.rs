use super::*;

#[test]
fn permanent_paper_deletion_cleans_owned_templates_without_exposing_them_globally() {
    let mut db = database();
    let target = paper(&db);
    let owned = ensure_paper_template(&db, "paper").unwrap();
    let published = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&owned.id, &template(), "owned-template"),
    )
    .unwrap()
    .resource;
    let global = super::super::legacy::imported_template(
        &db,
        "global-keep",
        "Global",
        &json!([{"id":"question","title":"Question","question":"Question?"}]),
    )
    .unwrap();
    let mut input = draft(&owned.id, "{draft survives only until purge", "owned-draft");
    input.expected_revision = published.revision;
    assert_eq!(
        crate::hbs_storage::test_sources::write(&mut db, input)
            .unwrap()
            .status,
        "invalid"
    );
    db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('owned-run','paper_overview','needs_review','fixture-generation','fixture-v1','v1','v1',1,?1,1000,1000,5000,'now','now')",["a".repeat(64)]).unwrap();
    db.execute("insert into ai_resource_template_runs(run_id,paper_id,template_id,template_revision,selection_revision,template_json) values('owned-run','paper',?1,?2,1,'{}')",rusqlite::params![owned.id,published.revision]).unwrap();
    db.execute(
        "update bibliographic_items set trashed_at='now' where id='paper'",
        [],
    )
    .unwrap();
    assert!(get(&db, &owned.id).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from structured_paper_template_owners where template_id=?1",
            [&owned.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    db.execute("delete from bibliographic_items where id='paper'", [])
        .unwrap();
    assert!(get(&db, &target.id).is_err());
    assert!(get(&db, &owned.id).is_err());
    assert!(get(&db, &global.id).is_ok());
    for table in [
        "structured_resource_drafts",
        "structured_resource_operations",
        "structured_resource_write_intents",
        "structured_resource_receipts",
    ] {
        let count: i64 = db
            .query_row(
                &format!("select count(*) from {table} where resource_id=?1"),
                [&owned.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(
        db.query_row(
            "select count(*) from ai_resource_template_runs where run_id='owned-run'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
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
fn raw_write_ahead_survives_prevalidation_crash_and_resumes_the_same_intent_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("phase-one.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let target = paper(&db);
    let input = draft(
        &target.id,
        &reading("崩溃前已持久化的内容。"),
        "resume-intent",
    );
    let fingerprint = hash(&encode(&input).unwrap());
    let staged = super::super::write_ahead::stage(&mut db, &input, &fingerprint).unwrap();
    assert!(query_outcome(&db, &input.request_id).unwrap().is_none());
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        encode(
            &read_text(&db, &target.id, Some("test-conversation"), false)
                .unwrap()
                .draft
                .unwrap()
        )
        .unwrap(),
        encode(&staged).unwrap()
    );
    let applied = crate::hbs_storage::test_sources::write(&mut db, input.clone()).unwrap();
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.resource.revision, 1);
    assert_eq!(
        encode(&crate::hbs_storage::test_sources::write(&mut db, input).unwrap()).unwrap(),
        encode(&applied).unwrap()
    );
    let count:i64=db.query_row("select count(*) from structured_resource_write_intents where request_id='resume-intent'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn failed_publication_retains_draft_and_never_reports_a_false_receipt() {
    let mut db = database();
    let target = paper(&db);
    db.execute_batch("create trigger reject_test_receipt before insert on structured_resource_receipts when new.request_id='fail-publish' begin select raise(abort,'injected publication failure');end;").unwrap();
    let failed = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("保存失败但内容保留。"), "fail-publish"),
    )
    .unwrap();
    assert_eq!(failed.status, "invalid");
    assert_eq!(failed.errors[0].code, "storage_error");
    assert!(failed.receipt.is_none());
    assert_eq!(get(&db, &target.id).unwrap().revision, 0);
    assert_eq!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .text,
        reading("保存失败但内容保留。")
    );
    assert!(
        applied_content_for_submission(&db, "test-conversation", "submission", &target.id)
            .unwrap()
            .is_none()
    );
    db.execute_batch("drop trigger reject_test_receipt;")
        .unwrap();
    let mut input = draft(
        &target.id,
        &reading("保存失败但内容保留。"),
        "retry-publication",
    );
    input.expected_draft_revision = Some(failed.draft.unwrap().revision);
    let published = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
    assert_eq!(published.status, "applied");
    assert!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .draft
            .is_none()
    );
    assert_eq!(
        applied_content_for_submission(&db, "test-conversation", "submission", &target.id).unwrap(),
        published.resource.content
    );
}

#[test]
fn paper_template_saves_diff_without_mutating_reading_binding() {
    let mut db = database();
    let reading_resource = paper(&db);
    let target = ensure_paper_template(&db, "paper").unwrap();
    let result = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &template(), "template-diff"),
    )
    .unwrap();
    assert_eq!(result.status, "applied");
    assert!(!result.resource.pending_field_ids.is_empty());
    assert!(
        get(&db, &reading_resource.id)
            .unwrap()
            .template_ref
            .is_none()
    );
    let kept = decision(&mut db, &result.resource, "keep", Some("all"), None);
    assert!(kept.pending_field_ids.is_empty());
}

#[test]
fn concurrent_reading_edit_is_not_overwritten_by_template_save() {
    let mut db = database();
    let reading_resource = paper(&db);
    let target = ensure_paper_template(&db, "paper").unwrap();
    let current = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(
            &reading_resource.id,
            &reading("同时编辑正文。"),
            "reading-edit",
        ),
    )
    .unwrap()
    .resource;
    let result = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &template(), "template-save"),
    )
    .unwrap();
    assert_eq!(result.status, "applied");
    assert_eq!(get(&db, &current.id).unwrap().content, current.content);
}
