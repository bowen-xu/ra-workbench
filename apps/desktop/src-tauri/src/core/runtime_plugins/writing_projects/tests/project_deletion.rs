use super::files::{seed, store};
use super::*;
use crate::{
    compile_activity::CompileManager,
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        writing_projects::files::{FileRequest, FileResult, operate_files},
    },
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

async fn delete(
    fixture: &Fixture,
    store: &AppStore,
    manager: &CompileManager,
    id: &str,
) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        id.into(),
        FileRequest::DeleteProject {
            manager: manager.clone(),
        },
    )
    .await
}
fn archive(fixture: &Fixture) {
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update manuscripts set status='archived' where id=?1",
            [&fixture.active],
        )
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_project_delete_sql_read_failure_keeps_the_available_native_identity() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let manager = CompileManager::default();
    seed(&store, &fixture.active, "main.tex", "root");
    archive(&fixture);
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "manuscript_compile_runs",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.active-compile"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.operation_id.is_some());
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "root"
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_project_delete_uses_original_manager_before_id_archive_and_sql_compile_checks()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let manager = CompileManager::default();
    let cancellation = manager.register("memory-only", "invalid-id").unwrap();
    let error = delete(&fixture, &store, &manager, "invalid-id")
        .await
        .unwrap_err();
    assert_eq!(error.code, "compile_already_running");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    manager.cancel("memory-only").unwrap();
    assert!(cancellation.load(std::sync::atomic::Ordering::Acquire));
    manager.finish("memory-only");
    let error = delete(&fixture, &store, &manager, "invalid-id")
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    seed(&store, &fixture.active, "main.tex", "root");
    let error = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(error.message, "请先归档稿件，再永久删除");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    archive(&fixture);
    fixture.database.lock().unwrap().execute("insert into manuscript_compile_runs(id,manuscript_id,engine,root_path,status,created_at) values('persisted-run',?1,'pdflatex','main.tex','queued','now')",[&fixture.active]).unwrap();
    let error = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(error.code, "compile_already_running");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .is_ok()
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_project_delete_removes_original_roots_and_cascades_only_owned_rows() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let manager = CompileManager::default();
    seed(&store, &fixture.active, "main.tex", "root");
    seed(&store, &fixture.archived, "main.tex", "other");
    let cache = store.writing_cache_directory().join(&fixture.active);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("output"), "cache").unwrap();
    {
        let db = fixture.database.lock().unwrap();
        db.execute(
            "insert into writing_sessions values(?1,'[\"main.tex\"]','main.tex','{}','{}','now')",
            [&fixture.active],
        )
        .unwrap();
        db.execute(
            "insert into manuscript_recovery_drafts values(?1,'main.tex',?2,'unsaved','now')",
            rusqlite::params![fixture.active, "a".repeat(64)],
        )
        .unwrap();
    }
    archive(&fixture);
    assert!(matches!(
        delete(&fixture, &store, &manager, &format!(" {} ", fixture.active))
            .await
            .unwrap(),
        FileResult::Deleted
    ));
    assert!(!store.manuscript_directory().join(&fixture.active).exists());
    assert!(!cache.exists());
    assert_eq!(
        store
            .manuscript_files(&fixture.archived)
            .read("main.tex")
            .unwrap()
            .0,
        "other"
    );
    {
        let db = fixture.database.lock().unwrap();
        for table in [
            "manuscripts",
            "manuscript_files",
            "writing_sessions",
            "manuscript_recovery_drafts",
        ] {
            let field = if table == "manuscripts" {
                "id"
            } else {
                "manuscript_id"
            };
            let count = db
                .query_row(
                    &format!("select count(*) from {table} where {field}=?1"),
                    [&fixture.active],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
    }
    let missing = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_project_delete_keeps_archived_record_on_sql_failure_and_accepts_missing_roots_on_retry()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let manager = CompileManager::default();
    seed(&store, &fixture.active, "main.tex", "root");
    archive(&fixture);
    fixture.database.lock().unwrap().execute_batch("create table synthetic_project_delete_prefix(value text);create temp trigger test_project_delete_fail before delete on manuscripts begin insert into synthetic_project_delete_prefix values('prefix');select raise(fail,'PRIVATE_DELETE_FAILURE');end;").unwrap();
    let error = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.delete"
    );
    assert!(!error.message.contains("PRIVATE_DELETE_FAILURE"));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!store.manuscript_directory().join(&fixture.active).exists());
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select status from manuscripts where id=?1",
                [&fixture.active],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "archived"
        );
        assert_eq!(
            db.query_row(
                "select count(*) from synthetic_project_delete_prefix",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger test_project_delete_fail")
            .unwrap();
    }
    assert!(matches!(
        delete(&fixture, &store, &manager, &fixture.active)
            .await
            .unwrap(),
        FileResult::Deleted
    ));
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_project_delete_preflights_both_roots_before_removing_source() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let manager = CompileManager::default();
    seed(&store, &fixture.active, "main.tex", "root");
    archive(&fixture);
    let cache = store.writing_cache_directory().join(&fixture.active);
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "not a directory").unwrap();
    let error = delete(&fixture, &store, &manager, &fixture.active)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "root"
    );
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), "not a directory");
    fixture.service.shutdown().await.unwrap();
}
