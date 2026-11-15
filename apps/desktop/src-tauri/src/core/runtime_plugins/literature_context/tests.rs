use super::*;
use crate::ai::structured_resource_validation::FailureCause;
use research_workbench_native_sdk::paper_context::PaperContextItem;
use std::{fs, path::Path};
pub(in crate::runtime_plugins) fn resources() -> (tempfile::TempDir, PaperContextResources) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("literature");
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(
        dir.join("manifest.json"),
        r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(dir.join("entry.js"), "export {};").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for id in 0..15 {
        let file = format!("paper-context-{}.v1.sql", specs::name(id));
        fs::copy(source.join(&file), dir.join("data").join(file)).unwrap();
    }
    for name in [
        "preview-header",
        "preview-items-budget",
        "preview-attachments-budget",
        "session-latest",
        "session-summary",
    ] {
        let file = format!("paper-context-{name}.v1.sql");
        fs::copy(source.join(&file), dir.join("data").join(file)).unwrap();
    }
    for name in ["snapshot-hash", "snapshot-rows"] {
        let file = format!("reading-evidence-{name}.v1.sql");
        fs::copy(source.join(&file), dir.join("data").join(file)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let package = catalog.discover().unwrap().packages.remove(0);
    let resources = PaperContextResources::load(&catalog, &package).unwrap();
    (root, resources)
}
pub(in crate::runtime_plugins) fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at)values('paper','journalArticle','Fixture','now','now');").unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)values(?1,'fixture.pdf','application/pdf',1,'now')",["a".repeat(64)]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at)values('attachment','paper',?1,'fixture.pdf','now')",["a".repeat(64)]).unwrap();
    db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at)values('parse','paper','attachment',?1,'http://127.0.0.1:1','success','success',' {\"texts\": []} ','{}','now','now')",["a".repeat(64)]).unwrap();
    db
}
fn projection() -> SnapshotProjectionV1 {
    SnapshotProjectionV1 {
        context_schema_version: 1,
        context_hash: "c".repeat(64),
        snapshot_id: format!("paper-context-{}", "c".repeat(64)),
        attachments: vec![PaperContextAttachment {
            attachment_id: "attachment".into(),
            content_sha256: "a".repeat(64),
            parse_run_id: "parse".into(),
            ordinal: 0,
        }],
        items: ["one", "two"]
            .into_iter()
            .map(|id| PaperContextItem {
                id: id.into(),
                attachment_id: "attachment".into(),
                content_sha256: "a".repeat(64),
                parse_run_id: "parse".into(),
                raw_ref: format!("#/texts/{id}"),
                kind: "text".into(),
                label: None,
                page_index: Some(0),
                regions: vec![],
                charspan: None,
                source_text_hash: None,
                text: Some(id.into()),
                table_json: None,
                image_content_sha256: None,
            })
            .collect(),
    }
}
fn insert(
    resources: &PaperContextResources,
    db: &mut Connection,
    p: &SnapshotProjectionV1,
) -> PaperContextSnapshot {
    let captured = resources.capture_snapshot(db, p).unwrap();
    resources
        .apply_snapshot(db, "paper", p, &captured, None)
        .unwrap()
}
#[test]
fn resolves_every_requested_attachment_before_parse_and_preserves_raw_witness() {
    let (_root, r) = resources();
    let db = database();
    db.execute_batch("pragma ignore_check_constraints=on;")
        .unwrap();
    db.execute("update docling_parse_runs set json_content='{'", [])
        .unwrap();
    let error = r
        .capture_build(&db, "paper", &["attachment".into(), "missing".into()])
        .unwrap_err();
    assert_eq!(error.failure.error.to_string(), "来源 PDF 不属于当前文献");
    db.execute_batch("pragma ignore_check_constraints=off;")
        .unwrap();
    db.execute(
        "update docling_parse_runs set json_content=' {\"texts\": []} '",
        [],
    )
    .unwrap();
    let first = r.capture_build(&db, "paper", &[]).unwrap();
    db.execute(
        "update docling_parse_runs set json_content='{\"texts\": []}'",
        [],
    )
    .unwrap();
    let second = r.capture_build(&db, "paper", &[]).unwrap();
    assert_eq!(first.input, second.input);
    assert_ne!(first.witness, second.witness);
    let repeated = r
        .capture_build(&db, "paper", &["attachment".into(), "attachment".into()])
        .unwrap();
    assert_eq!(repeated.input.attachments.len(), 2);
}
#[test]
fn only_missing_latest_parse_consults_stale_facts() {
    let (_root, r) = resources();
    let db = database();
    let first = r.capture_build(&db, "paper", &[]).unwrap();
    assert!(!first.input.attachments[0].has_stale_parse);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)values(?1,'old.pdf','application/pdf',1,'now')",["b".repeat(64)]).unwrap();
    db.execute(
        "update docling_parse_runs set content_sha256=?1",
        ["b".repeat(64)],
    )
    .unwrap();
    let stale = r.capture_build(&db, "paper", &[]).unwrap();
    assert!(stale.input.attachments[0].parse.is_none());
    assert!(stale.input.attachments[0].has_stale_parse);
}
#[test]
fn per_row_json_failure_precedes_later_sql_codec_failure_and_i64_is_not_narrowed() {
    let (_root, r) = resources();
    let mut db = database();
    let p = projection();
    insert(&r, &mut db, &p);
    db.execute(
        "update paper_context_items set regions_json='[\"bad\"]' where context_item_id='one'",
        [],
    )
    .unwrap();
    db.execute(
        "update paper_context_items set raw_ref=x'00' where context_item_id='two'",
        [],
    )
    .unwrap();
    let error = r.load_snapshot(&db, &p.snapshot_id).unwrap_err();
    assert!(matches!(error.failure.error, AiError::TemporaryUnavailable));
    assert!(matches!(error.failure.cause, FailureCause::Original));
    db.execute(
        "update paper_context_items set regions_json='[]' where context_item_id='one'",
        [],
    )
    .unwrap();
    let error = r.load_snapshot(&db, &p.snapshot_id).unwrap_err();
    assert!(matches!(error.failure.cause, FailureCause::SourceData(_)));
    db.execute(
        "update paper_context_items set raw_ref='fixed',page_index=4294967296",
        [],
    )
    .unwrap();
    assert!(
        r.load_snapshot(&db, &p.snapshot_id)
            .unwrap()
            .items
            .iter()
            .all(|item| item.page_index.is_none())
    );
}
#[test]
fn snapshot_witness_rejects_changed_raw_bytes_before_writes_and_same_hash_reuses_actual_id() {
    let (_root, r) = resources();
    let mut db = database();
    let p = projection();
    let snapshot = insert(&r, &mut db, &p);
    let capture = r.capture_snapshot(&db, &p).unwrap();
    let before = db.total_changes();
    assert_eq!(
        r.apply_snapshot(&mut db, "paper", &p, &capture, Some(&p.context_hash))
            .unwrap(),
        snapshot
    );
    assert_eq!(db.total_changes(), before);
    db.execute(
        "update paper_context_items set regions_json='[ ]' where context_item_id='one'",
        [],
    )
    .unwrap();
    let before = db.total_changes();
    assert!(
        r.apply_snapshot(&mut db, "paper", &p, &capture, Some(&p.context_hash))
            .is_err()
    );
    assert_eq!(db.total_changes(), before);
}
#[test]
fn protected_corrupt_snapshot_is_never_deleted() {
    let (_root, r) = resources();
    let mut db = database();
    let p = projection();
    insert(&r, &mut db, &p);
    db.execute("insert into paper_analysis_workflows(id,paper_id,context_snapshot_id,status,provider_id,model_id,prompt_set_version,created_at,updated_at)values('failed','paper',?1,'failed','fixture-generation','fixture-v1','v1','now','now')",[&p.snapshot_id]).unwrap();
    let captured = r.capture_snapshot(&db, &p).unwrap();
    let before = db.total_changes();
    let error = r
        .apply_snapshot(&mut db, "paper", &p, &captured, Some("different-digest"))
        .unwrap_err();
    assert_eq!(
        error.failure.error.to_string(),
        "已引用的论文快照校验失败，请重新解析当前 PDF 后重试"
    );
    assert_eq!(db.total_changes(), before);
    assert_eq!(
        r.load_snapshot(&db, &p.snapshot_id).unwrap(),
        captured.existing.unwrap()
    );
}
#[test]
fn delete_commits_before_separate_failed_insertion_and_reports_committed_partial_work() {
    let (_root, r) = resources();
    let mut db = database();
    let p = projection();
    insert(&r, &mut db, &p);
    let capture = r.capture_snapshot(&db, &p).unwrap();
    db.execute_batch("create trigger fail_context_insert before insert on paper_context_snapshots begin select raise(abort,'PRIVATE_DRIVER_TEXT');end;").unwrap();
    let error = r
        .apply_snapshot(&mut db, "paper", &p, &capture, Some("different-digest"))
        .unwrap_err();
    assert_eq!(error.outcome, HbsOutcome::Committed);
    assert!(matches!(error.failure.cause, FailureCause::SourceData(_)));
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from paper_context_items", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn first_insert_failure_rolls_back_header_and_attachments() {
    let (_root, r) = resources();
    let mut db = database();
    let p = projection();
    let capture = r.capture_snapshot(&db, &p).unwrap();
    db.execute_batch("create trigger fail_context_item before insert on paper_context_items begin select raise(abort,'PRIVATE_DRIVER_TEXT');end;").unwrap();
    let error = r
        .apply_snapshot(&mut db, "paper", &p, &capture, None)
        .unwrap_err();
    assert_eq!(error.outcome, HbsOutcome::RolledBack);
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from paper_context_attachments",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
