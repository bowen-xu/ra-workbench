#![cfg(unix)]
use super::*;
use std::sync::{Arc, atomic::Ordering};
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: std::time::Duration = std::time::Duration::from_secs(8);

#[tokio::test]
async fn normal_annotation_admission_waits_and_rechecks_stop_or_cleanup_without_replay() {
    use crate::data_interface::Database;
    for blocked in [
        None,
        Some("plugin_host_stopping"),
        Some("plugin_host_cleanup_blocked"),
    ] {
        let mut fixture = Fixture::existing().await;
        let observed = Arc::new(tokio::sync::Notify::new());
        Arc::get_mut(&mut fixture.service)
            .unwrap()
            .admission_observed = Some(Arc::clone(&observed));
        let held = fixture.service.host.lock().await;
        let instance = held.as_ref().unwrap().error("test", "test").instance_id;
        let service = Arc::clone(&fixture.service);
        let database = Arc::clone(&fixture.db);
        let key = fixture.key.clone();
        let request = tokio::spawn(async move {
            service
                .annotation_owned(database, &key, fixture::request())
                .await
        });
        timeout(WAIT, observed.notified()).await.unwrap();
        let waiting = !request.is_finished();
        let note = {
            let mut db = fixture.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            let mut document = crate::notes::load_document_value(&tx, fixture::PAGE).unwrap();
            let revision = tx
                .query_row(
                    "select revision from note_pages where id=?1",
                    [fixture::PAGE],
                    |row| row.get(0),
                )
                .unwrap();
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Core Note saved while NAL annotation waits","styles":[]}]});
            crate::notes::save_document_in(
                &tx,
                crate::notes::NoteDocumentSaveRequest {
                    document,
                    expected_revision: revision,
                    save_reason: "manual".into(),
                },
            )
            .unwrap();
            tx.commit().unwrap();
            serde_json::to_value(crate::notes::load_document_value(&db, fixture::PAGE).unwrap())
                .unwrap()
        };
        if blocked == Some("plugin_host_stopping") {
            fixture.service.stopping.store(true, Ordering::Release);
        }
        if blocked == Some("plugin_host_cleanup_blocked") {
            fixture
                .service
                .cleanup_failed
                .store(true, Ordering::Release);
        }
        drop(held);
        let result = timeout(WAIT, request).await.unwrap().unwrap();
        assert_eq!(
            fixture
                .service
                .host
                .lock()
                .await
                .as_ref()
                .unwrap()
                .error("test", "test")
                .instance_id,
            instance
        );
        fixture.service.stopping.store(false, Ordering::Release);
        fixture
            .service
            .cleanup_failed
            .store(false, Ordering::Release);
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        fixture.assert_cleaned().await;
        assert!(
            waiting,
            "normal annotation did not wait for its host: {result:?}"
        );
        if let Some(code) = blocked {
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(fixture.snapshot().flags, (0, 0, 0));
        } else {
            assert_eq!(result.unwrap(), fixture::annotation());
            assert_eq!(fixture.snapshot().flags, (1, 1, 2));
        }
        let committed = fixture.snapshot();
        let Fixture {
            data, db, service, ..
        } = fixture;
        drop(service);
        db.shutdown().unwrap();
        drop(db);
        let mut sql = rusqlite::Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let reopened = Database::open(sql, &data.path().join("data-log")).unwrap();
        {
            let db = reopened.lock().unwrap();
            assert_eq!(
                serde_json::to_value(
                    crate::notes::load_document_value(&db, fixture::PAGE).unwrap()
                )
                .unwrap(),
                note
            );
            let cold = fixture::snapshot(&db);
            assert_eq!(
                (cold.flags, cold.notes, cold.annotations),
                (committed.flags, committed.notes, committed.annotations)
            );
        }
        reopened.shutdown().unwrap();
    }
}

#[tokio::test]
async fn invalid_annotation_is_rejected_by_the_first_call_without_any_database_statement() {
    use rusqlite::hooks::{AuthContext, Authorization};
    use std::sync::atomic::AtomicUsize;

    let fixture = Fixture::invalid().await;
    let before = fixture.snapshot();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&calls);
        db.authorizer(Some(move |_: AuthContext<'_>| {
            observed.fetch_add(1, Ordering::SeqCst);
            Authorization::Allow
        }));
    }
    let mut invalid = fixture::request();
    invalid.target_id = " \t".into();
    let result = timeout(
        WAIT,
        fixture.service.ensure_nal_annotation(
            Arc::clone(&fixture.db),
            fixture.key.clone(),
            invalid,
        ),
    )
    .await;
    let observed = calls.load(Ordering::SeqCst);
    // Remove the observer before the test's own read-only snapshot/close work.
    fixture
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let error = result.unwrap().unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "笔记目标或标题无效");
    assert_eq!(observed, 0, "invalid input entered R0 or another SQL path");
    let diagnostic = serde_json::to_value(&error).unwrap();
    assert_eq!(diagnostic["annotationOutcome"]["status"], "notStarted");
    assert!(diagnostic["annotationOutcome"].get("revision").is_none());
    assert_eq!(fixture.snapshot(), before);
    assert!(fixture.service.host.lock().await.is_some());
    assert_eq!(
        std::fs::read_dir(&fixture.input_directory).unwrap().count(),
        0
    );
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.assert_cleaned().await;
    fixture.db.shutdown().unwrap();
}

#[tokio::test]
async fn wrong_third_operation_retires_the_host_after_r1_without_t1_writes() {
    let fixture = Fixture::wrong_third_operation().await;
    let before = fixture.snapshot();
    let error = timeout(
        WAIT,
        fixture.service.ensure_nal_annotation(
            Arc::clone(&fixture.db),
            fixture.key.clone(),
            fixture::request(),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.exit_code.is_some() || error.signal.is_some());
    assert!(
        error.cleanup_error.is_none(),
        "safe native cleanup diagnostic: {error:?}"
    );
    let diagnostic = serde_json::to_value(&error).unwrap();
    assert_eq!(diagnostic["annotationOutcome"]["status"], "notStarted");
    assert!(diagnostic["annotationOutcome"].get("revision").is_none());
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains(fixture.data.path().to_str().unwrap())
    );
    fixture.assert_cleaned().await;
    assert_eq!(
        fixture.snapshot(),
        before,
        "R1 rejection entered a write path"
    );
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}

#[tokio::test]
async fn aborted_existing_annotation_waiter_keeps_startup_t1_owned_until_shutdown_and_cold_reopen()
{
    use crate::data_interface::Database;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    // Only Existing plus pending empty startup flags is canned here. This is
    // ownership/settlement evidence, not the Create domain-engine acceptance.
    let mut fixture = Fixture::existing().await;
    let observed = Arc::new(tokio::sync::Notify::new());
    Arc::get_mut(&mut fixture.service)
        .unwrap()
        .admission_observed = Some(Arc::clone(&observed));
    let before = fixture.snapshot();
    assert_eq!(before.flags, (0, 0, 0));
    assert_eq!(before.notes.len(), 1);
    assert_eq!(before.annotations.len(), 1);
    assert!(fixture.db.logged());
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_annotation_startup_barrier",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                entered_tx
                    .send(())
                    .map_err(|cause| rusqlite::Error::UserFunctionError(Box::new(cause)))?;
                release_rx
                    .recv()
                    .map_err(|cause| rusqlite::Error::UserFunctionError(Box::new(cause)))?;
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger test_annotation_startup_pause before update of legacy_imported on nal_state begin select test_annotation_startup_barrier();end;").unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move {
        service
            .ensure_nal_annotation(database, key, fixture::request())
            .await
    });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();
    timeout(WAIT, observed.notified()).await.unwrap();
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let queued = tokio::spawn(async move {
        service
            .annotation_owned(database, &key, fixture::request())
            .await
    });
    timeout(WAIT, observed.notified()).await.unwrap();
    let queued_waiting = !queued.is_finished();
    client.abort();
    let cancelled = client.await.is_err_and(|cause| cause.is_cancelled());
    let retained_host = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    release_tx.send(()).unwrap();
    assert!(
        cancelled && retained_host,
        "outer abort released accepted T1"
    );
    assert!(stopping && pending, "shutdown did not wait for accepted T1");
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    let queued_error = timeout(WAIT, queued).await.unwrap().unwrap().unwrap_err();
    assert!(
        queued_waiting,
        "concurrent annotation did not wait for accepted SQL: {queued_error:?}"
    );
    assert_eq!(queued_error.code, "plugin_host_stopping");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    fixture.assert_cleaned().await;
    let committed = fixture.snapshot();
    assert_eq!(committed.flags, (1, 1, 2));
    assert_eq!(committed.notes, before.notes);
    assert_eq!(committed.annotations, before.annotations);
    assert_eq!(Arc::strong_count(&fixture.db), 1);
    assert_eq!(Arc::strong_count(&fixture.service), 1);

    // The cancelled client has no success receipt. Verify the actual original
    // SQL/log pair after releasing all old owners; do not seed or replay.
    let Fixture {
        data, db, service, ..
    } = fixture;
    drop(service);
    db.shutdown().unwrap();
    drop(db);
    let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened = Database::open(sql, &data.path().join("data-log")).unwrap();
    {
        let reopened = fixture::snapshot(&reopened.lock().unwrap());
        assert_eq!(reopened.flags, committed.flags);
        assert_eq!(reopened.notes, committed.notes);
        assert_eq!(reopened.annotations, committed.annotations);
    }
    reopened.shutdown().unwrap();
}
