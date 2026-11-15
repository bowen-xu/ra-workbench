use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome, writing_projects::metadata::update_project,
};
use research_workbench_native_sdk::writing_project::UpdateManuscriptRequest;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

async fn update(
    fixture: &Fixture,
    id: &str,
    title: Option<&str>,
    root_path: Option<&str>,
) -> NativeResult<Manuscript> {
    update_project(
        Arc::clone(&fixture.database),
        fixture.directory.path().join("manuscripts"),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        UpdateManuscriptRequest {
            manuscript_id: id.into(),
            title: title.map(str::to_owned),
            root_path: root_path.map(str::to_owned),
        },
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_updates_title_and_root_without_replacing_identity_or_requiring_an_indexed_root()
 {
    let fixture = Fixture::new();
    let original = super::creation::create(&fixture, uuid::Uuid::new_v4(), None, "Original")
        .await
        .unwrap();
    let source = fixture
        .directory
        .path()
        .join("manuscripts")
        .join(&original.id)
        .join("source");
    std::fs::write(
        source.join("sections/alternate.TEX"),
        "alternative UTF-8 source\n",
    )
    .unwrap();
    let changed = update(
        &fixture,
        &format!(" {} ", original.id),
        Some("  Updated title  "),
        Some("sections/alternate.TEX"),
    )
    .await
    .unwrap();
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.created_at, original.created_at);
    assert_eq!(changed.project_id, original.project_id);
    assert_eq!(changed.title, "Updated title");
    assert_eq!(changed.root_path, "sections/alternate.TEX");
    let unchanged = update(&fixture, &original.id, None, None).await.unwrap();
    assert_eq!(unchanged.title, changed.title);
    assert_eq!(unchanged.root_path, changed.root_path);
    assert_eq!(
        fixture
            .database
            .lock()
            .unwrap()
            .query_row(
                "select relative_path from manuscript_files where manuscript_id=?1",
                [&original.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "sections/main.tex"
    );
    // Title-only updates keep the original no-file-read path and normalization.
    let normalized = update(&fixture, &fixture.active, None, None).await.unwrap();
    assert_eq!(normalized.title, "Legacy title");
    assert_eq!(normalized.root_path, "main.tex");
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_root_checks_preserve_io_kind_missing_project_and_title_error_order() {
    let fixture = Fixture::new();
    let missing = uuid::Uuid::new_v4().to_string();
    let source = fixture
        .directory
        .path()
        .join("manuscripts")
        .join(&missing)
        .join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("entry.md"), "text").unwrap();
    std::fs::write(source.join("entry.tex"), "text").unwrap();
    std::fs::write(source.join("binary.tex"), [0xff]).unwrap();
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "manuscripts",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let kind = update(&fixture, &missing, Some(""), Some("entry.md"))
        .await
        .unwrap_err();
    assert_eq!(kind.code, "invalid_input");
    assert_eq!(kind.message, "稿件路径无效");
    let io = update(&fixture, &missing, Some(""), Some("missing.tex"))
        .await
        .unwrap_err();
    assert_eq!(io.code, "not_found");
    assert_eq!(io.message, "稿件文件不存在");
    let encoding = update(&fixture, &missing, Some(""), Some("binary.tex"))
        .await
        .unwrap_err();
    assert_eq!(encoding.code, "unsupported_encoding");
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let absent = update(&fixture, &missing, Some(""), Some("entry.tex"))
        .await
        .unwrap_err();
    assert_eq!(absent.code, "not_found");
    assert_eq!(absent.message, "稿件不存在");
    let title = update(&fixture, &fixture.active, Some(""), None)
        .await
        .unwrap_err();
    assert_eq!(title.code, "invalid_input");
    assert_eq!(title.message, "稿件信息无效");
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_metadata_failure_keeps_original_autocommit_prefix() {
    let fixture = Fixture::new();
    fixture.database.lock().unwrap().execute_batch("create table synthetic_metadata_prefix(value text not null);
        create temp trigger test_metadata_fail before update of title on manuscripts
        begin insert into synthetic_metadata_prefix values('committed prefix'); select raise(fail,'PRIVATE_METADATA_FAILURE');end;").unwrap();
    let error = update(&fixture, &fixture.active, Some("Requested title"), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.update"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!error.message.contains("PRIVATE_METADATA_FAILURE"));
    let db = fixture.database.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select title from manuscripts where id=?1",
            [&fixture.active],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "  Legacy title  "
    );
    assert_eq!(
        db.query_row(
            "select count(*) from synthetic_metadata_prefix",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_metadata_rechecks_full_rows_after_native_and_rejects_concurrent_change() {
    let fixture = Fixture::new();
    let first_read = Arc::new(tokio::sync::Notify::new());
    let (release_read, read_release) = std::sync::mpsc::channel();
    let seen = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.database.lock().unwrap();
        db.execute(
            "update manuscripts set title=?2 where id=?1",
            rusqlite::params![fixture.active, "Initial title"],
        )
        .unwrap();
        let attempts = Arc::clone(&writes);
        db.create_scalar_function(
            "test_metadata_requested_write",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger test_metadata_requested_write before update of title on manuscripts
            when new.title='Requested title' begin select test_metadata_requested_write();end;").unwrap();
        let signal = Arc::clone(&first_read);
        db.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "manuscripts",
                    ..
                }
            ) && !seen.swap(true, Ordering::SeqCst)
            {
                // Queue the second Core writer while the actual capture owns
                // the connection, then release capture before cold Native init.
                signal.notify_one();
                if read_release.recv_timeout(Duration::from_secs(90)).is_err() {
                    return Authorization::Deny;
                }
            }
            Authorization::Allow
        }));
    }
    let database = Arc::clone(&fixture.database);
    let service = Arc::clone(&fixture.service);
    let catalog = Arc::clone(&fixture.catalog);
    let key = fixture.key.clone();
    let id = fixture.active.clone();
    let directory = fixture.directory.path().join("manuscripts");
    let updating = tokio::spawn(async move {
        update_project(
            database,
            directory,
            service,
            catalog,
            key,
            UpdateManuscriptRequest {
                manuscript_id: id,
                title: Some("Requested title".into()),
                root_path: None,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(90), first_read.notified())
        .await
        .unwrap();
    let concurrent_db = Arc::clone(&fixture.database);
    let concurrent_id = fixture.active.clone();
    let writer_entered = Arc::new(tokio::sync::Notify::new());
    let entering = Arc::clone(&writer_entered);
    let concurrent = tokio::task::spawn_blocking(move || {
        // The owner can be reacquired during the actual Native calculation.
        // Leave updated_at unchanged so a timestamp-only check would miss it.
        entering.notify_one();
        concurrent_db
            .lock()
            .unwrap()
            .execute(
                "update manuscripts set title='Concurrent title' where id=?1",
                [concurrent_id],
            )
            .unwrap();
    });
    tokio::time::timeout(Duration::from_secs(90), writer_entered.notified())
        .await
        .unwrap();
    release_read.send(()).unwrap();
    concurrent.await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(90), updating)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    let diagnostic = error.data_error.unwrap();
    assert_eq!(diagnostic.resource_id, "writing.projects.get");
    assert_eq!(diagnostic.phase, "recheck");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    let db = fixture.database.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select title,updated_at from manuscripts where id=?1",
            [&fixture.active],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        )
        .unwrap(),
        ("Concurrent title".into(), "updated".into())
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}
