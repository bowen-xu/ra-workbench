use super::*;
use crate::runtime_plugins::writing_projects::sessions::{get_session, save_session};
use research_workbench_native_sdk::writing_project::WritingSessionDraft;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(30);

fn draft(id: &str) -> WritingSessionDraft {
    WritingSessionDraft {
        manuscript_id: id.into(),
        open_paths: vec!["main.tex".into(), "sections/方法.tex".into()],
        // The original API does not require active_path to belong to open_paths.
        active_path: Some("notes/thoughts.ranote".into()),
        cursor_state: serde_json::json!({"main.tex": {"anchor": 4, "head": 8}}),
        layout_state: serde_json::json!({"preview": false, "width": 0.3}),
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_session_roundtrip_preserves_original_rows_errors_and_reopen() {
    let fixture = Fixture::new();
    assert!(
        get_session(
            Arc::clone(&fixture.database),
            &fixture.service,
            Arc::clone(&fixture.catalog),
            fixture.key.clone(),
            fixture.active.clone(),
        )
        .await
        .unwrap()
        .is_none()
    );
    let original = draft(&fixture.active);
    save_session(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        original.clone(),
    )
    .await
    .unwrap();
    let loaded = get_session(
        Arc::clone(&fixture.database),
        &fixture.service,
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        format!(" {} ", fixture.active),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(loaded.manuscript_id, original.manuscript_id);
    assert_eq!(loaded.open_paths, original.open_paths);
    assert_eq!(loaded.active_path, original.active_path);
    assert_eq!(loaded.cursor_state, original.cursor_state);
    assert_eq!(loaded.layout_state, original.layout_state);
    let mut invalid = original.clone();
    invalid.open_paths.push("../escape.tex".into());
    invalid.manuscript_id = "missing".into();
    let rejected = save_session(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        invalid,
    )
    .await
    .unwrap_err();
    assert_eq!(rejected.code, "invalid_input");
    assert_eq!(rejected.message, "稿件路径无效");
    let missing = save_session(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        draft("missing"),
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "storage_error");
    assert_eq!(
        missing.data_error.unwrap().sqlite_code.as_deref(),
        Some("ConstraintViolation")
    );
    assert!(missing.instance_id.is_some() && missing.request_id.is_some());
    // Legacy stored JSON errors remain storage failures, with a safe cause.
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update writing_sessions set open_paths_json='null' where manuscript_id=?1",
            [&fixture.active],
        )
        .unwrap();
    let malformed = get_session(
        Arc::clone(&fixture.database),
        &fixture.service,
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        fixture.active.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(malformed.code, "storage_error");
    assert!(malformed.message.contains("open paths JSON"));
    save_session(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        original,
    )
    .await
    .unwrap();
    fixture.service.shutdown().await.unwrap();
    fixture.database.shutdown().unwrap();
    drop(fixture.database);
    let sql =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    let reopened = Database::open(sql, &fixture.directory.path().join("data-log")).unwrap();
    let restarted = NativePreviewService::new(Arc::clone(&fixture.catalog));
    let after = get_session(
        reopened,
        &restarted,
        fixture.catalog,
        fixture.key,
        fixture.active,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(after.open_paths, loaded.open_paths);
    assert_eq!(after.active_path, loaded.active_path);
    assert_eq!(after.cursor_state, loaded.cursor_state);
    assert_eq!(after.layout_state, loaded.layout_state);
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_session_write_survives_waiter_abort_and_shutdown_drains_once() {
    let fixture = Fixture::new();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.database.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_writing_session_pause",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(WAIT).unwrap();
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger test_writing_session_pause before insert on writing_sessions begin select test_writing_session_pause();end;").unwrap();
    }
    let db = Arc::clone(&fixture.database);
    let service = Arc::clone(&fixture.service);
    let catalog = Arc::clone(&fixture.catalog);
    let key = fixture.key.clone();
    let original = draft(&fixture.active);
    let expected = original.clone();
    let client =
        tokio::spawn(async move { save_session(db, service, catalog, key, original).await });
    timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    let service = Arc::clone(&fixture.service);
    let mut shutdown = tokio::spawn(async move { service.shutdown().await });
    timeout(WAIT, async {
        loop {
            let rejected = fixture
                .service
                .project_decision(
                    fixture.key.clone(),
                    Input::PrepareSession {
                        draft: expected.clone(),
                    },
                )
                .await;
            if rejected.is_err_and(|error| error.code == "plugin_host_stopping") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let premature = timeout(Duration::from_millis(100), &mut shutdown).await;
    release_tx.send(()).unwrap();
    assert!(
        premature.is_err(),
        "shutdown released storage before the active SQL finished"
    );
    timeout(WAIT, shutdown).await.unwrap().unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let db = fixture.database.lock().unwrap();
    let (paths, active): (String, Option<String>) = db
        .query_row(
            "select open_paths_json, active_path from writing_sessions where manuscript_id=?1",
            [&fixture.active],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&paths).unwrap(),
        expected.open_paths
    );
    assert_eq!(active, expected.active_path);
}
