use super::*;
use super::{
    file_save::note,
    files::{seed, store},
};
use crate::{
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        writing_projects::files::catalog_preparation::prepare_catalog,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_catalog::{Preparation, PreparationScope};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

async fn prepare(
    fixture: &Fixture,
    store: &AppStore,
    retry: bool,
    scope: PreparationScope,
) -> NativeResult<Preparation> {
    prepare_catalog(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        retry,
        scope,
    )
    .await
}
fn source(store: &AppStore, id: &str, path: &str) -> std::path::PathBuf {
    store
        .manuscript_directory()
        .join(id)
        .join("source")
        .join(path)
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_keeps_eight_file_scope_and_original_note_engine_body() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    for index in 0..10 {
        let path = format!("notes/{index:02}.ranote");
        let content = note(&format!("active-{index}"), "RareBody 研究");
        seed(&store, &fixture.active, &path, &content);
    }
    for index in 0..3 {
        seed(
            &store,
            &fixture.archived,
            &format!("notes/{index}.ranote"),
            &note(&format!("archived-{index}"), "Archived"),
        );
    }
    let first = prepare(&fixture, &store, false, PreparationScope::Active)
        .await
        .unwrap();
    assert_eq!(
        (
            first.processed_count,
            first.pending_count,
            first.error_count
        ),
        (8, 2, 0)
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(db.query_row("select count(*) from writing_note_catalog where manuscript_id=?1 and state='pending'",[&fixture.archived],|row|row.get::<_,i64>(0)).unwrap(),3);
        let body:(String,String)=db.query_row("select search_text,folded_text from writing_note_catalog_body where manuscript_id=?1 and relative_path='notes/00.ranote'",[&fixture.active],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(
            body,
            (
                "RareBody 研究".into(),
                research_note_query::note_fold("RareBody 研究")
            )
        );
    }
    let next = prepare(&fixture, &store, false, PreparationScope::Active)
        .await
        .unwrap();
    assert_eq!(
        (next.processed_count, next.pending_count, next.error_count),
        (2, 0, 0)
    );
    let archived = prepare(&fixture, &store, false, PreparationScope::Archived)
        .await
        .unwrap();
    assert_eq!(
        (
            archived.processed_count,
            archived.pending_count,
            archived.error_count
        ),
        (3, 0, 0)
    );
    std::fs::remove_file(source(&store, &fixture.active, "notes/00.ranote")).unwrap();
    let empty = prepare(&fixture, &store, false, PreparationScope::Live)
        .await
        .unwrap();
    assert_eq!(
        (
            empty.processed_count,
            empty.pending_count,
            empty.error_count
        ),
        (0, 0, 0)
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_preserves_duplicate_invalid_missing_and_external_file_results()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let content = note("stable", "Original body");
    seed(&store, &fixture.active, "a.ranote", &content);
    seed(&store, &fixture.active, "b.ranote", &content);
    seed(&store, &fixture.active, "c.ranote", "not json");
    seed(
        &store,
        &fixture.active,
        "d.ranote",
        &note("missing", "removed"),
    );
    std::fs::remove_file(source(&store, &fixture.active, "d.ranote")).unwrap();
    let result = prepare(&fixture, &store, false, PreparationScope::Live)
        .await
        .unwrap();
    assert_eq!(
        (
            result.processed_count,
            result.pending_count,
            result.error_count
        ),
        (4, 0, 3)
    );
    {
        let db = fixture.database.lock().unwrap();
        for (path, prefix) in [
            ("b.ranote", "writing_note_identity_duplicate"),
            ("c.ranote", "writing_note_json_invalid"),
            (
                "d.ranote",
                "writing_note_catalog_read_failed：稿件文件不存在",
            ),
        ] {
            let cause=db.query_row("select error_cause from writing_note_catalog where manuscript_id=?1 and relative_path=?2",rusqlite::params![fixture.active,path],|row|row.get::<_,String>(0)).unwrap();
            assert!(cause.starts_with(prefix), "{cause}");
        }
        db.execute("update writing_note_catalog set state='error',error_cause='fixture' where manuscript_id=?1 and relative_path='a.ranote'",[&fixture.active]).unwrap();
    }
    let external = note("stable", "External body");
    std::fs::write(source(&store, &fixture.active, "a.ranote"), &external).unwrap();
    let result = prepare(&fixture, &store, true, PreparationScope::Active)
        .await
        .unwrap();
    assert_eq!(
        (
            result.processed_count,
            result.pending_count,
            result.error_count
        ),
        (4, 0, 4)
    );
    let db = fixture.database.lock().unwrap();
    let cause=db.query_row("select error_cause from writing_note_catalog where manuscript_id=?1 and relative_path='a.ranote'",[&fixture.active],|row|row.get::<_,String>(0)).unwrap();
    assert!(cause.starts_with("writing_note_catalog_external_change"));
    assert_eq!(db.query_row("select search_text from writing_note_catalog_body where manuscript_id=?1 and relative_path='a.ranote'",[&fixture.active],|row|row.get::<_,String>(0)).unwrap(),"Original body");
    assert_eq!(
        std::fs::read_to_string(source(&store, &fixture.active, "a.ranote")).unwrap(),
        external
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_commits_each_file_and_rolls_back_only_the_failed_file()
{
    let fixture = Fixture::new();
    let store = store(&fixture);
    for name in ["a", "b", "c"] {
        seed(
            &store,
            &fixture.active,
            &format!("{name}.ranote"),
            &note(name, name),
        );
    }
    fixture.database.lock().unwrap().execute_batch("create table synthetic_catalog_prefix(value text);create temp trigger test_catalog_fail before update on writing_note_catalog when new.relative_path='b.ranote' and new.state='ready' begin insert into synthetic_catalog_prefix values('prefix');select raise(fail,'PRIVATE_CATALOG_FAILURE');end;").unwrap();
    let error = prepare(&fixture, &store, false, PreparationScope::Live)
        .await
        .unwrap_err();
    assert_eq!(error.code, "index_failed");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert!(
        error
            .message
            .contains("writing_note_catalog_prepare_failed")
    );
    assert!(!error.message.contains("PRIVATE_CATALOG_FAILURE"));
    assert!(
        error.instance_id.is_some() && error.request_id.is_some() && error.operation_id.is_some()
    );
    {
        let db = fixture.database.lock().unwrap();
        let states = db
            .prepare("select relative_path,state from writing_note_catalog order by relative_path")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            states,
            [
                ("a.ranote".into(), "ready".into()),
                ("b.ranote".into(), "pending".into()),
                ("c.ranote".into(), "pending".into())
            ]
        );
        assert_eq!(
            db.query_row("select count(*) from synthetic_catalog_prefix", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger test_catalog_fail").unwrap();
    }
    let retried = prepare(&fixture, &store, false, PreparationScope::Live)
        .await
        .unwrap();
    assert_eq!(
        (
            retried.processed_count,
            retried.pending_count,
            retried.error_count
        ),
        (2, 0, 0)
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_releases_database_and_skips_a_concurrently_removed_registration()
 {
    let fixture = Arc::new(Fixture::new());
    let store = store(&fixture);
    seed(
        &store,
        &fixture.active,
        "note.ranote",
        &note("stable", "old"),
    );
    let captured = Arc::new(tokio::sync::Notify::new());
    let (release, waiting) = std::sync::mpsc::channel();
    {
        let signal = Arc::clone(&captured);
        let seen = AtomicBool::new(false);
        fixture
            .database
            .lock()
            .unwrap()
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "writing_note_catalog",
                        ..
                    }
                ) && !seen.swap(true, Ordering::SeqCst)
                {
                    signal.notify_one();
                    if waiting.recv_timeout(Duration::from_secs(90)).is_err() {
                        return Authorization::Deny;
                    }
                }
                Authorization::Allow
            }));
    }
    let borrowed = Arc::clone(&fixture);
    let worker_store = store.clone();
    let preparing = tokio::spawn(async move {
        prepare(&borrowed, &worker_store, false, PreparationScope::Live).await
    });
    tokio::time::timeout(Duration::from_secs(90), captured.notified())
        .await
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let entering = Arc::clone(&entered);
    let database = Arc::clone(&fixture.database);
    let id = fixture.active.clone();
    let writer = tokio::task::spawn_blocking(move || {
        entering.notify_one();
        database.lock().unwrap().execute("delete from manuscript_files where manuscript_id=?1 and relative_path='note.ranote'",[id]).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(90), entered.notified())
        .await
        .unwrap();
    release.send(()).unwrap();
    writer.await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(90), preparing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            result.processed_count,
            result.pending_count,
            result.error_count
        ),
        (0, 0, 0)
    );
    assert_eq!(
        fixture
            .database
            .lock()
            .unwrap()
            .query_row("select count(*) from writing_note_catalog", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert!(source(&store, &fixture.active, "note.ranote").exists());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_unconfirmed_rollback_fences_database_and_keeps_authoritative_file()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let content = note("stable", "Original body");
    seed(&store, &fixture.active, "note.ranote", &content);
    fixture.database.lock().unwrap().execute_batch("create temp trigger test_catalog_rollback before update on writing_note_catalog when new.state='ready' begin select raise(rollback,'PRIVATE_CATALOG_ROLLBACK');end;").unwrap();
    let error = prepare(&fixture, &store, false, PreparationScope::Live)
        .await
        .unwrap_err();
    assert_eq!(error.code, "index_failed");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert!(error.save_rollback_error.is_some());
    assert!(!error.message.contains("PRIVATE_CATALOG_ROLLBACK"));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(store.connection().is_err());
    assert_eq!(
        std::fs::read_to_string(source(&store, &fixture.active, "note.ranote")).unwrap(),
        content
    );
    let reopened =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .query_row("select state from writing_note_catalog", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "pending"
    );
    assert_eq!(
        reopened
            .query_row(
                "select count(*) from writing_note_catalog_body",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_catalog_preparation_retry_failure_keeps_the_original_autocommit_prefix() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(
        &store,
        &fixture.active,
        "note.ranote",
        &note("stable", "body"),
    );
    fixture.database.lock().unwrap().execute_batch("update writing_note_catalog set state='error',error_cause='fixture';create table synthetic_retry_prefix(value text);create temp trigger test_catalog_retry_fail before update on writing_note_catalog when old.state='error' and new.state='pending' begin insert into synthetic_retry_prefix values('prefix');select raise(fail,'PRIVATE_RETRY_FAILURE');end;").unwrap();
    let error = prepare(&fixture, &store, true, PreparationScope::Active)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.note-prepare-retry"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!error.message.contains("PRIVATE_RETRY_FAILURE"));
    {
        let db = fixture.database.lock().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("select count(*) from synthetic_retry_prefix", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("select state from writing_note_catalog", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "error"
        );
        db.execute_batch("drop trigger test_catalog_retry_fail")
            .unwrap();
    }
    let retried = prepare(&fixture, &store, true, PreparationScope::Active)
        .await
        .unwrap();
    assert_eq!(
        (
            retried.processed_count,
            retried.pending_count,
            retried.error_count
        ),
        (1, 0, 0)
    );
    fixture.service.shutdown().await.unwrap();
}
