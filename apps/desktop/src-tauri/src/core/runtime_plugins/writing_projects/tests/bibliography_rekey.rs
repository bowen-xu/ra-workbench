//! Actual independent host migration across source files, canonical rows and HBS baselines.
use super::bibliography_runtime::list;
use super::files::{seed, store};
use super::*;
use crate::runtime_plugins::writing_projects::files::{
    FileRequest, FileResult, bibliography::Call, operate_files,
};
use research_workbench_native_sdk::writing_bibliography::rekey::{Assignment, Migration};
use rusqlite::params;
use serde_json::json;

async fn migrate(
    f: &Fixture,
    store: &AppStore,
    assignments: Vec<Assignment>,
) -> NativeResult<Migration> {
    match operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Rekey { assignments },
        },
    )
    .await?
    {
        FileResult::CitationKeyMigration(migration) => Ok(*migration),
        _ => panic!("migration result"),
    }
}
async fn prepare(f: &Fixture, store: &AppStore) -> Vec<Assignment> {
    let text = "\\cite{x,x2} revised";
    let hash = seed(store, &f.active, "main.tex", text);
    seed(store, &f.active, "other.tex", "\\cite{x}");
    seed(
        store,
        &f.active,
        "refs.bib",
        "@article{x,title={Alpha},year={2026}}\n@article{x2,title={Beta},year={2026}}",
    );
    {
        let db = f.database.lock().unwrap();
        db.execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'",[&f.active]).unwrap();
        let checkpoint = json!({"schemaVersion":1,"manuscriptId":f.active,"relativePath":"main.tex","version":1,
            "content":text,"reviewBaseline":"\\cite{x,x2} original","savedSha256":hash,"savedVersion":1,"operations":[],"saveIntent":null});
        db.execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at) values(?1,'main.tex',1,?2,'original')",params![f.active,checkpoint.to_string()]).unwrap();
    }
    list(f, store)
        .await
        .unwrap()
        .into_iter()
        .map(|row| Assignment {
            reference_id: row.id,
            base_key: "x".into(),
            from: row.citation_key.clone(),
            to: if row.citation_key == "x" {
                "x:a"
            } else {
                "x:b"
            }
            .into(),
        })
        .collect()
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_rekeys_sources_bibliography_review_and_lost_reply_then_reopens() {
    let f = Fixture::new();
    let store = store(&f);
    let assignments = prepare(&f, &store).await;
    let result = migrate(&f, &store, assignments.clone()).await.unwrap();
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.checkpoints[0].revision, 2);
    assert_eq!(
        result.checkpoints[0].checkpoint.content,
        "\\cite{x:a,x:b} revised"
    );
    assert_eq!(
        result.checkpoints[0].checkpoint.review_baseline,
        "\\cite{x:a,x:b} original"
    );
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("other.tex")
            .unwrap()
            .0,
        "\\cite{x:a}"
    );
    let rows = list(&f, &store).await.unwrap();
    for assignment in &assignments {
        let row = rows
            .iter()
            .find(|row| row.id == assignment.reference_id)
            .unwrap();
        assert_eq!(row.citation_key, assignment.to);
        assert_eq!(row.base_key.as_deref(), Some("x"));
    }
    let retry = migrate(&f, &store, assignments.clone()).await.unwrap();
    assert!(retry.files.is_empty());
    assert_eq!(retry.checkpoints, result.checkpoints);
    let pending: i64 = f
        .database
        .lock()
        .unwrap()
        .query_row(
            "select count(*) from manuscript_citation_key_writes where manuscript_id=?1",
            [&f.active],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    f.service.shutdown().await.unwrap();
    let service = Arc::new(NativePreviewService::new(Arc::clone(&f.catalog)));
    let FileResult::CitationKeyMigration(reopened) = operate_files(
        store,
        Arc::clone(&service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Rekey { assignments },
        },
    )
    .await
    .unwrap() else {
        panic!("reopened migration");
    };
    assert!(reopened.files.is_empty());
    assert_eq!(reopened.checkpoints, result.checkpoints);
    service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_rekey_late_sql_failure_rolls_back_checkpoints_keys_and_redo_then_retries() {
    let f = Fixture::new();
    let store = store(&f);
    let assignments = prepare(&f, &store).await;
    f.database.lock().unwrap().execute_batch("create trigger reject_rekey before insert on manuscript_citation_key_bases begin select raise(abort,'synthetic rekey base failure'); end;").unwrap();
    let error = migrate(&f, &store, assignments.clone()).await.unwrap_err();
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.query_database_error.is_some());
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        "\\cite{x,x2} revised"
    );
    let db = f.database.lock().unwrap();
    assert!(db.is_autocommit());
    let revision: u64 = db
        .query_row(
            "select revision from writing_document_checkpoints where manuscript_id=?1",
            [&f.active],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
    let keys:Vec<String>=db.prepare("select citation_key from manuscript_bibliography_entries where manuscript_id=?1 order by citation_key").unwrap().query_map([&f.active],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(keys, ["x", "x2"]);
    let pending: i64 = db
        .query_row(
            "select count(*) from manuscript_citation_key_writes where manuscript_id=?1",
            [&f.active],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    db.execute_batch("drop trigger reject_rekey").unwrap();
    drop(db);
    assert_eq!(
        migrate(&f, &store, assignments).await.unwrap().files.len(),
        2
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_rekey_keeps_file_conflict_before_checkpoint_json_and_pending_save_rejection()
 {
    let f = Fixture::new();
    let store = store(&f);
    let assignments = prepare(&f, &store).await;
    let saved = store.manuscript_files(&f.active).read("main.tex").unwrap();
    let checkpoint:String=f.database.lock().unwrap().query_row("select checkpoint_json from writing_document_checkpoints where manuscript_id=?1 and relative_path='main.tex'",[&f.active],|row|row.get(0)).unwrap();
    f.database
        .lock()
        .unwrap()
        .execute(
            "update writing_document_checkpoints set checkpoint_json='[]' where manuscript_id=?1",
            [&f.active],
        )
        .unwrap();
    store
        .manuscript_files(&f.active)
        .save("main.tex", "external file change", Some(&saved.1))
        .unwrap();
    assert_eq!(
        migrate(&f, &store, assignments.clone())
            .await
            .unwrap_err()
            .code,
        "file_conflict"
    );
    store
        .manuscript_files(&f.active)
        .save("main.tex", &saved.0, None)
        .unwrap();
    let json = migrate(&f, &store, assignments.clone()).await.unwrap_err();
    assert_eq!(json.code, "storage_error");
    assert!(json.message.contains("序列化"));
    let mut pending: serde_json::Value = serde_json::from_str(&checkpoint).unwrap();
    pending["saveIntent"] = json!({"operationId":"pending-save","version":1,"expectedSha256":saved.1,"targetSha256":"b".repeat(64)});
    f.database
        .lock()
        .unwrap()
        .execute(
            "update writing_document_checkpoints set checkpoint_json=?2 where manuscript_id=?1",
            params![f.active, pending.to_string()],
        )
        .unwrap();
    assert_eq!(
        migrate(&f, &store, assignments).await.unwrap_err().code,
        "checkpoint_conflict"
    );
    let db = f.database.lock().unwrap();
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row(
            "select revision from writing_document_checkpoints where manuscript_id=?1",
            [&f.active],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let keys:Vec<String>=db.prepare("select citation_key from manuscript_bibliography_entries where manuscript_id=?1 order by citation_key").unwrap().query_map([&f.active],|row|row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(keys, ["x", "x2"]);
    drop(db);
    f.service.shutdown().await.unwrap();
}
