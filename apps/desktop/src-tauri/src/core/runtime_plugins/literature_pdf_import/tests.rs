//! Core SQL/IO stages use real fixed resources and the original database owner.
//! Native preparation/projection and the background worker are separate checks.
use super::*;
use crate::{
    data_interface::Database,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_attachments::AttachmentResources,
        literature_catalog::{
            data::CatalogResources,
            save::{SaveOutcome, SaveResources},
        },
        literature_import_jobs::ImportJobsResources,
    },
    storage::AppStore,
};
use execute::{PdfStorage, SaveStage};
use research_workbench_native_sdk::{
    catalog_save::{CatalogSaveOutcome, CatalogSavePlan},
    pdf_import::{PdfImportCandidate, PreparedPdfImportEntryV1},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{fs, path::Path};

const PDF: &[u8] = b"%PDF-1.7\n% synthetic finite PDF import\n%%EOF\n";
struct Fixture {
    directory: tempfile::TempDir,
    storage: PdfStorage,
}
fn open(path: &Path) -> AppStore {
    fs::create_dir_all(path.join("objects")).unwrap();
    let mut sql = Connection::open(path.join("store.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    AppStore::for_test_database(
        Database::open(sql, &path.join("data-log")).unwrap(),
        path.join("objects"),
    )
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = open(directory.path());
        let root = directory.path().join("packages/literature");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(root.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        for file in fs::read_dir(source).unwrap() {
            let file = file.unwrap();
            if file
                .path()
                .extension()
                .is_some_and(|extension| extension == "sql")
            {
                fs::copy(file.path(), root.join("data").join(file.file_name())).unwrap();
            }
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let storage = PdfStorage {
            store,
            pdf: PdfImportResources::load(&catalog, &package).unwrap(),
            jobs: ImportJobsResources::load(&catalog, &package).unwrap(),
            reads: CatalogResources::load(&catalog, &package).unwrap(),
            writes: SaveResources::load(&catalog, &package).unwrap(),
            attachments: AttachmentResources::load(&catalog, &package).unwrap(),
        };
        Self { directory, storage }
    }
    fn entry(&self, name: &str, bytes: &[u8]) -> PreparedPdfImportEntryV1 {
        let path = self.directory.path().join(name);
        fs::write(&path, bytes).unwrap();
        let candidate: PdfImportCandidate = serde_json::from_value(json!({"path":path,"draft":{"id":"original-unconsumed-id","itemType":"document","title":" Original\u{0} title ","sourceKind":"user"}})).unwrap();
        PreparedPdfImportEntryV1 {
            input_label: candidate.path.clone(),
            raw_input: serde_json::to_string(&candidate).unwrap(),
            candidate,
        }
    }
    fn snapshot(&self, job: &str) -> Value {
        let db = self.storage.store.connection().unwrap();
        let header = db
            .query_row(
                "select state,completed_count,cancellation_requested from import_jobs where id=?1",
                [job],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        let results = db.prepare("select state,item_id,error_message,raw_input from import_results where job_id=?1 order by created_at,rowid").unwrap()
            .query_map([job], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,String>(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        json!({"header":header,"results":results})
    }
    fn reopen(self) -> Self {
        let Self { directory, storage } = self;
        let PdfStorage {
            store,
            pdf,
            jobs,
            reads,
            writes,
            attachments,
        } = storage;
        store.shared_agent_connection().shutdown().unwrap();
        // Release the original Raft engine before opening the same directory.
        drop(store);
        let storage = PdfStorage {
            store: open(directory.path()),
            pdf,
            jobs,
            reads,
            writes,
            attachments,
        };
        Self { directory, storage }
    }
}
fn plan() -> CatalogSaveOutcome {
    // An already prepared synthetic snapshot, not a second domain planner.
    let plan: CatalogSavePlan = serde_json::from_value(json!({
        "item":{"id":null,"itemType":"document","title":"Synthetic imported PDF","abstractText":"","issuedYear":null,"containerTitle":"",
            "conferenceName":"","proceedingsTitle":"","place":"","volume":"","issue":"","pages":"","url":"","language":"","bibtexKey":"","rawBibtex":null},
        "creators":[],"doi":null,"tags":[],"provenance":{"fields":["title"],"sourceKind":"user","confirmedByUser":true},
        "duplicates":{"normalizedTitle":"synthetic imported pdf","doiReason":"doi","doiConfidence":1.0,"titleReason":"normalized_title_year","titleConfidence":0.72}
    })).unwrap();
    CatalogSaveOutcome::Planned {
        plan: Box::new(plan),
    }
}

#[test]
fn queued_header_item_autocommits_mixed_finish_and_cold_reopen_preserve_original_input() {
    let f = Fixture::new();
    let valid = f.entry("valid.pdf", PDF);
    let invalid = f.entry("invalid.pdf", b"not a PDF");
    let entries = [valid, invalid];
    let (job, _, _) = execute::create(&f.storage, &entries).unwrap();
    assert_eq!(f.snapshot(&job)["header"], json!(["queued", 0, 0]));
    execute::mark_running(&f.storage, &job).unwrap();
    let pending = execute::pending(&f.storage, &job).unwrap();
    for (index, row) in pending.into_iter().enumerate() {
        assert_eq!(row.raw_input, entries[index].raw_input);
        assert!(!execute::observe_cancel(&f.storage, &job).unwrap());
        let mut progress = execute::progress(&job, "test-save");
        progress.result_id = Some(row.id.clone());
        let SaveStage::Saved(saved) = execute::save(&f.storage, plan(), &mut progress).unwrap()
        else {
            panic!("expected saved rows");
        };
        assert!(
            !saved.items[0].main.rows.is_empty(),
            "complete committed readback retained"
        );
        let outcome = execute::attach(
            &f.storage,
            &saved.item_id,
            &entries[index].candidate.path,
            &mut progress,
        )
        .unwrap();
        execute::record(&f.storage, &row.id, outcome, index, &mut progress).unwrap();
    }
    execute::finish(&f.storage, &job).unwrap();
    let before = f.snapshot(&job);
    assert_eq!(before["header"], json!(["completed_with_errors", 2, 0]));
    assert_eq!(before["results"][0][0], "imported");
    assert_eq!(before["results"][1][0], "failed");
    assert!(
        before["results"][1][2]
            .as_str()
            .unwrap()
            .contains("不是有效的 PDF 文件")
    );
    assert_eq!(
        f.storage
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from bibliographic_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let reopened = f.reopen();
    assert_eq!(reopened.snapshot(&job), before);
}

#[test]
fn cancellation_after_false_observation_finishes_current_item_then_cancels_pending_without_aggregate()
 {
    let f = Fixture::new();
    let entry = f.entry("source.pdf", PDF);
    let (job, _, _) = execute::create(&f.storage, &[entry.clone(), entry.clone()]).unwrap();
    execute::mark_running(&f.storage, &job).unwrap();
    let pending = execute::pending(&f.storage, &job).unwrap();
    assert!(!execute::observe_cancel(&f.storage, &job).unwrap());
    f.storage
        .store
        .connection()
        .unwrap()
        .execute(
            "update import_jobs set cancellation_requested=1 where id=?1",
            [&job],
        )
        .unwrap();
    let mut progress = execute::progress(&job, "test-save");
    let SaveStage::Saved(saved) = execute::save(&f.storage, plan(), &mut progress).unwrap() else {
        panic!("expected saved");
    };
    let outcome = execute::attach(
        &f.storage,
        &saved.item_id,
        &entry.candidate.path,
        &mut progress,
    )
    .unwrap();
    execute::record(&f.storage, &pending[0].id, outcome, 0, &mut progress).unwrap();
    assert!(execute::observe_cancel(&f.storage, &job).unwrap());
    let snapshot = f.snapshot(&job);
    assert_eq!(snapshot["header"], json!(["cancelled", 1, 1]));
    assert_eq!(snapshot["results"][0][0], "imported");
    assert_eq!(snapshot["results"][1][0], "cancelled");
}

#[test]
fn committed_save_readback_failure_remains_an_item_failure_without_attachment_or_parent_delete() {
    let f = Fixture::new();
    let entry = f.entry("source.pdf", PDF);
    let (job, _, _) = execute::create(&f.storage, &[entry]).unwrap();
    f.storage.store.connection().unwrap().execute_batch("create trigger broken_pdf_readback after insert on bibliographic_items begin update bibliographic_items set issued_year='not-an-integer' where id=new.id; end;").unwrap();
    let mut p = execute::progress(&job, "test-save");
    let SaveStage::Failed(message) = execute::save(&f.storage, plan(), &mut p).unwrap() else {
        panic!("readback must fail");
    };
    assert!(matches!(p.last_save, Some(SaveOutcome::Committed { .. })));
    assert!(message.contains("文献数据库操作失败"));
    let db = f.storage.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from bibliographic_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_parent_delete_supersedes_attachment_error_only_in_persisted_business_content() {
    let f = Fixture::new();
    let entry = f.entry("invalid.pdf", b"not PDF");
    let (job, _, _) = execute::create(&f.storage, std::slice::from_ref(&entry)).unwrap();
    let mut p = execute::progress(&job, "test-save");
    let SaveStage::Saved(saved) = execute::save(&f.storage, plan(), &mut p).unwrap() else {
        panic!("saved");
    };
    f.storage.store.connection().unwrap().execute_batch("create trigger deny_pdf_parent_delete before delete on bibliographic_items begin select raise(FAIL,'synthetic-private-parent-delete'); end;").unwrap();
    let outcome =
        execute::attach(&f.storage, &saved.item_id, &entry.candidate.path, &mut p).unwrap();
    assert!(
        outcome
            .as_ref()
            .unwrap_err()
            .contains("synthetic-private-parent-delete")
    );
    assert!(
        !serde_json::to_string(&p)
            .unwrap()
            .contains("synthetic-private-parent-delete")
    );
    let row = execute::pending(&f.storage, &job).unwrap().remove(0);
    execute::record(&f.storage, &row.id, outcome, 0, &mut p).unwrap();
    assert!(
        f.snapshot(&job)["results"][0][2]
            .as_str()
            .unwrap()
            .contains("synthetic-private-parent-delete")
    );
}

#[test]
fn result_failure_does_not_increment_progress_and_fail_attempts_both_original_updates() {
    let f = Fixture::new();
    let entry = f.entry("source.pdf", PDF);
    let (job, _, _) = execute::create(&f.storage, &[entry]).unwrap();
    let row = execute::pending(&f.storage, &job).unwrap().remove(0);
    f.storage.store.connection().unwrap().execute_batch("create trigger deny_pdf_result before update on import_results when new.state='imported' begin select raise(FAIL,'synthetic-private-result'); end;").unwrap();
    let mut p = execute::progress(&job, "test-result");
    let error =
        execute::record(&f.storage, &row.id, Ok("synthetic-item".into()), 0, &mut p).unwrap_err();
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("synthetic-private-result")
    );
    assert_eq!(f.snapshot(&job)["header"][1], 0);
    f.storage.store.connection().unwrap().execute_batch("create trigger deny_pdf_fail_header before update on import_jobs when new.state='failed' begin select raise(FAIL,'synthetic-private-fail-header'); end;").unwrap();
    execute::fail(&f.storage, &job, &error).unwrap_err();
    assert_eq!(f.snapshot(&job)["results"][0][0], "failed");
    assert!(
        f.snapshot(&job)["results"][0][2]
            .as_str()
            .unwrap()
            .contains("synthetic-private-result")
    );
}

#[test]
fn original_pending_primitive_failure_precedes_cancellation_and_is_not_normalized_to_empty() {
    let f = Fixture::new();
    let entry = f.entry("source.pdf", PDF);
    let (job, _, _) = execute::create(&f.storage, &[entry]).unwrap();
    f.storage
        .store
        .connection()
        .unwrap()
        .execute(
            "update import_results set raw_input=null where job_id=?1",
            params![job],
        )
        .unwrap();
    let error = execute::pending(&f.storage, &job).unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(error.data_error.is_some());
    assert_eq!(
        f.storage
            .store
            .connection()
            .unwrap()
            .query_row("select state from import_jobs where id=?1", [&job], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "queued"
    );
}

#[test]
fn fenced_owner_stops_job_bookkeeping_without_relabeling_it_failed_or_replaying() {
    let f = Fixture::new();
    let entry = f.entry("source.pdf", PDF);
    let (job, _, _) = execute::create(&f.storage, &[entry]).unwrap();
    f.storage.store.connection().unwrap().require_recovery();
    let error = execute::mark_running(&f.storage, &job).unwrap_err();
    execute::fail(&f.storage, &job, &error).unwrap_err();
    // Read the exact synthetic file independently after the owner rejected both
    // operations; no direct write stands in for a production recovery action.
    let db = Connection::open(f.directory.path().join("store.sqlite3")).unwrap();
    let state: String = db
        .query_row("select state from import_jobs where id=?1", [&job], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(state, "queued");
    let rows: i64 = db
        .query_row("select count(*) from bibliographic_items", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}
