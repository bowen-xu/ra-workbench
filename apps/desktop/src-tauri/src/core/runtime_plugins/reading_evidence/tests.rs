use super::*;
use std::{fs, path::Path};

fn resources() -> (tempfile::TempDir, ReadingEvidenceResources) {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("literature");
    fs::create_dir_all(package.join("data")).unwrap();
    fs::write(
        package.join("manifest.json"),
        r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(package.join("entry.js"), "export {};").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in [
        "reading-evidence-snapshot-hash.v1.sql",
        "reading-evidence-snapshot-rows.v1.sql",
    ] {
        fs::copy(source.join(name), package.join("data").join(name)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let package = catalog.discover().unwrap().packages.remove(0);
    let resources = ReadingEvidenceResources::load(&catalog, &package).unwrap();
    (root, resources)
}
fn snapshot_db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table paper_context_snapshots(id,paper_id,context_hash);create table paper_context_items(snapshot_id,context_item_id,attachment_id,content_sha256,page_index,text_content,table_json);create table attachments(id,content_sha256);create table ai_evidence_bundles(id,run_id);insert into paper_context_snapshots values('snapshot','paper','original-hash');insert into attachments values('present','current-sha');insert into paper_context_items values('snapshot','b','missing','frozen-b',null,null,'{\"b\":2}');insert into paper_context_items values('snapshot','a','present','frozen-a',-2,'Original exact text','ignored table');").unwrap();
    db
}

#[test]
fn exact_optional_hash_and_ordered_rows_keep_missing_attachment_and_i64_semantics() {
    let (_root, r) = resources();
    let db = snapshot_db();
    assert_eq!(
        r.snapshot_hash(&db, "snapshot", "paper")
            .unwrap()
            .as_deref(),
        Some("original-hash")
    );
    assert_eq!(r.snapshot_hash(&db, "snapshot", "other").unwrap(), None);
    assert_eq!(r.snapshot_hash(&db, " snapshot", "paper").unwrap(), None);
    assert_eq!(r.snapshot_evidence_rows(&db, "absent").unwrap(), vec![]);
    let rows = r.snapshot_evidence_rows(&db, "snapshot").unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "a".into(),
                "present".into(),
                "frozen-a".into(),
                -2,
                "Original exact text".into(),
                "current-sha".into()
            ),
            (
                "b".into(),
                "missing".into(),
                "frozen-b".into(),
                0,
                "{\"b\":2}".into(),
                "".into()
            )
        ]
    );
    let bundle =
        crate::ai::structured_resource_validation::explicit_evidence(&r, &db, "paper", "snapshot")
            .unwrap();
    assert_eq!(bundle.items[0].current_anchor_status, "stale");
    assert_eq!(bundle.items[1].current_anchor_status, "missing");
    assert_eq!(bundle.items[0].quote_exact, "Original exact text");
    let error =
        crate::ai::structured_resource_validation::explicit_evidence(&r, &db, "other", "snapshot")
            .unwrap_err();
    assert_eq!(error.to_string(), "冻结来源不存在或属于其他文献");
    assert!(error.data.is_none());
}

#[test]
fn actual_bad_later_row_refuses_whole_result_with_safe_data_cause() {
    let (_root, r) = resources();
    let db = snapshot_db();
    db.execute(
        "update paper_context_items set page_index='bad integer' where context_item_id='b'",
        [],
    )
    .unwrap();
    let error =
        crate::ai::structured_resource_validation::explicit_evidence(&r, &db, "paper", "snapshot")
            .unwrap_err();
    assert_eq!(error.to_string(), "无法读取冻结原文");
    let cause = error.data.unwrap();
    assert_eq!(
        cause.resource_id,
        "literature.reading-evidence.snapshot-rows"
    );
    assert_eq!(cause.package_key, r.package_key());
    assert!(!cause.safe_cause.is_empty());
}

#[test]
fn actual_bad_hash_preserves_original_read_failure_instead_of_missing_source() {
    let (_root, r) = resources();
    let db = snapshot_db();
    db.execute("update paper_context_snapshots set context_hash=x'00'", [])
        .unwrap();
    let error =
        crate::ai::structured_resource_validation::explicit_evidence(&r, &db, "paper", "snapshot")
            .unwrap_err();
    assert_eq!(error.to_string(), "无法读取冻结来源");
    assert_eq!(
        error.data.unwrap().resource_id,
        "literature.reading-evidence.snapshot-hash"
    );
}

#[test]
fn original_core_startup_import_and_once_only_marker_use_real_two_read_facet() {
    let (_root, r) = resources();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Synthetic paper','now','now');insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('unit','paper','research_question','Question','Original manual body.','question','now','now');").unwrap();
    let draft = || crate::structured_resources::StartupImportDraft {
        catalog: None,
        presentations: vec![],
        contexts: vec![],
    };
    let pending =
        crate::structured_resources::import_startup_with_evidence(None, &mut db, draft(), None)
            .unwrap();
    assert_eq!(pending.deferred_paper_ids, ["paper"]);
    let first =
        crate::structured_resources::import_startup_with_evidence(Some(&r), &mut db, draft(), None)
            .unwrap();
    assert_eq!(first.imported_paper_count, 1);
    assert!(first.deferred_paper_ids.is_empty());
    let before:String=db.query_row("select content_json from structured_resources where paper_id='paper' and kind='structuredReading'",[],|row|row.get(0)).unwrap();
    assert!(before.contains("Original manual body."));
    db.execute("update structured_reading_units set body='Later legacy body must not reproject' where id='unit'",[]).unwrap();
    let again =
        crate::structured_resources::import_startup_with_evidence(Some(&r), &mut db, draft(), None)
            .unwrap();
    assert_eq!(again.imported_paper_count, 0);
    let after:String=db.query_row("select content_json from structured_resources where paper_id='paper' and kind='structuredReading'",[],|row|row.get(0)).unwrap();
    assert_eq!(after, before);
}
