#![cfg(unix)]
use super::*;
use std::sync::{Arc, atomic::Ordering};
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: std::time::Duration = std::time::Duration::from_secs(8);

#[tokio::test]
async fn normal_mutation_admission_waits_and_rechecks_stop_or_cleanup_without_replay() {
    for blocked in [
        None,
        Some("plugin_host_stopping"),
        Some("plugin_host_cleanup_blocked"),
    ] {
        let mut fixture = Fixture::new(false).await;
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
                .mutation_owned(database, &key, fixture::mutation(), 2)
                .await
        });
        timeout(WAIT, observed.notified()).await.unwrap();
        // The current-thread owned body has reached the mutex, not a timer.
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
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Core Note saved while NAL mutation waits","styles":[]}]});
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
        // These flags were injected only to test the post-lock admission check.
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
            "normal mutation did not wait for its host: {result:?}"
        );
        if let Some(code) = blocked {
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(fixture.snapshot().flags, (0, 0, 0));
        } else {
            assert_eq!(result.unwrap().revision, 3);
            assert_eq!(fixture.snapshot().flags, (1, 1, 3));
        }
        let committed = fixture.snapshot();
        assert_eq!(committed.document, note);
        assert_eq!(fixture.cold_snapshot(false), committed);
    }
}

fn assert_identity(error: &NativeHostError, fixture: &Fixture) {
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    let diagnostic = serde_json::to_string(error).unwrap();
    for private in [
        fixture.data.path().to_str().unwrap(),
        "Synthetic preserved mutation service Core body",
        "synthetic-private-absent-annotation",
    ] {
        assert!(!diagnostic.contains(private));
    }
}

#[tokio::test]
async fn wrong_second_reply_retires_before_t1_and_only_explicit_next_call_restarts() {
    let fixture = Fixture::new(true).await;
    let before = fixture.snapshot();
    let changes = fixture.db.lock().unwrap().total_changes();
    let mut previous_instance = None;
    // Two explicit caller invocations, not an internal retry loop. Both canned
    // replies reject the phase; neither is a domain recovery-success claim.
    for _ in 0..2 {
        let error = timeout(
            WAIT,
            fixture.service.mutate_nal_network(
                Arc::clone(&fixture.db),
                fixture.key.clone(),
                fixture::mutation(),
                2,
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.code, "plugin_host_response_mismatch");
        assert_identity(&error, &fixture);
        assert_ne!(error.request_id.as_ref(), Some(&fixture.initial_identity.1));
        assert_eq!(error.signal, Some(9));
        let diagnostic = serde_json::to_value(&error).unwrap();
        assert_eq!(diagnostic["mutationOutcome"]["status"], "notStarted");
        if let Some(previous) = &previous_instance {
            assert_ne!(error.instance_id.as_ref().unwrap(), previous);
        }
        previous_instance = error.instance_id;
        fixture.assert_cleaned().await;
        assert_eq!(fixture.snapshot(), before);
        assert_eq!(
            fixture.db.lock().unwrap().total_changes(),
            changes,
            "wrong second phase entered T1"
        );
    }
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}

#[tokio::test]
async fn aborted_noop_waiter_keeps_accepted_t1_owned_until_shutdown_and_cold_reopen() {
    use rusqlite::functions::FunctionFlags;
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    // Empty-import preparation plus an absent-link no-op is canned here. The
    // actual Core startup, CAS, mutation SQL, final load and commit still run.
    let mut fixture = Fixture::new(false).await;
    let observed = Arc::new(tokio::sync::Notify::new());
    Arc::get_mut(&mut fixture.service)
        .unwrap()
        .admission_observed = Some(Arc::clone(&observed));
    let before = fixture.snapshot();
    assert_eq!(before.flags, (0, 0, 0));
    assert!(fixture.db.logged());
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_mutation_startup_barrier",
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
        db.execute_batch("create temp trigger test_mutation_startup_pause before update of legacy_imported on nal_state begin select test_mutation_startup_barrier();end;").unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move {
        service
            .mutate_nal_network(database, key, fixture::mutation(), 2)
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
            .mutation_owned(database, &key, fixture::mutation(), 3)
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
        "outer abort released accepted mutation T1"
    );
    assert!(
        stopping && pending,
        "shutdown did not wait for accepted mutation T1"
    );
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    let queued_error = timeout(WAIT, queued).await.unwrap().unwrap().unwrap_err();
    assert!(
        queued_waiting,
        "concurrent mutation did not wait for accepted SQL: {queued_error:?}"
    );
    assert_eq!(queued_error.code, "plugin_host_stopping");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    fixture.assert_cleaned().await;
    let committed = fixture.snapshot();
    assert_eq!(committed.flags, (1, 1, 3));
    assert_eq!(committed.document, before.document);
    assert_eq!(committed.counts, before.counts);
    // The aborted waiter has no success receipt. Close every prior owner and
    // inspect the actual SQLite/log pair without seeding or replaying a call.
    assert_eq!(fixture.cold_snapshot(false), committed);
}

#[tokio::test]
async fn rollback_and_unknown_keep_safe_identity_and_never_acknowledge_uncertain_commit() {
    use rusqlite::functions::FunctionFlags;
    use std::sync::atomic::AtomicUsize;

    // Distinct fresh fixtures, not retries: CAS conflict confirms rollback;
    // a failing actual COMMIT hook deliberately requires an Unknown fence.
    for unknown in [false, true] {
        let fixture = Fixture::new(false).await;
        let before = fixture.snapshot();
        let commit_attempts = Arc::new(AtomicUsize::new(0));
        if unknown {
            let db = fixture.db.lock().unwrap();
            let observed = Arc::clone(&commit_attempts);
            db.create_scalar_function(
                "observe_mutation_final_revision",
                0,
                FunctionFlags::SQLITE_UTF8,
                move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(0_i64)
                },
            )
            .unwrap();
            db.execute_batch("create temp trigger observe_mutation_final_revision before update of revision on nal_state when old.revision=2 and new.revision=3 begin select observe_mutation_final_revision();end;").unwrap();
            db.commit_hook(Some(|| true));
        }
        let error = timeout(
            WAIT,
            fixture.service.mutate_nal_network(
                Arc::clone(&fixture.db),
                fixture.key.clone(),
                fixture::mutation(),
                if unknown { 2 } else { 99 },
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_identity(&error, &fixture);
        let diagnostic = serde_json::to_value(&error).unwrap();
        if unknown {
            assert_eq!(error.code, "storage_error");
            assert_eq!(error.message, "关系修改提交或回滚结果未知，禁止自动重试。");
            assert_eq!(diagnostic["mutationOutcome"]["status"], "unknown");
            assert_eq!(diagnostic["mutationOutcome"]["observedRevision"], 3);
            assert_eq!(diagnostic["mutationProblem"]["phase"], "commit");
            assert_eq!(commit_attempts.load(Ordering::SeqCst), 1);
            assert!(
                fixture.db.lock().is_err(),
                "unknown commit lost original recovery fence"
            );
            // A new explicit caller attempt is blocked at its original owner
            // before any new exchange. The prior request must not label it.
            let owner_error = timeout(
                WAIT,
                fixture.service.mutate_nal_network(
                    Arc::clone(&fixture.db),
                    fixture.key.clone(),
                    fixture::mutation(),
                    2,
                ),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert_eq!(owner_error.code, "storage_error");
            assert_eq!(
                owner_error.package_key.as_deref(),
                Some(fixture.key.as_str())
            );
            assert_eq!(owner_error.instance_id, error.instance_id);
            assert!(owner_error.request_id.is_none());
            assert!(owner_error.cleanup_error.is_none(), "{owner_error:?}");
            assert_eq!(
                serde_json::to_value(&owner_error).unwrap()["mutationOutcome"]["status"],
                "notStarted"
            );
            assert_eq!(commit_attempts.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(error.code, "version_conflict");
            assert_eq!(error.message, "关系已变化，请刷新后重试");
            assert_eq!(diagnostic["mutationOutcome"]["status"], "rolledBack");
            assert_eq!(diagnostic["mutationProblem"]["phase"], "revisionCheck");
            assert_eq!(fixture.snapshot(), before);
        }
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        fixture.assert_cleaned().await;
        assert_eq!(fixture.cold_snapshot(unknown), before);
    }
}

#[tokio::test]
async fn shutdown_before_first_probe_clears_init_request_after_actual_retirement() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    let fixture = Fixture::stopped_before_probe().await;
    let before = fixture.snapshot();
    let changes = fixture.db.lock().unwrap().total_changes();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reads = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&reads);
        db.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "nal_state",
                    column_name: "legacy_imported",
                    ..
                }
            ) && observed.fetch_add(1, Ordering::SeqCst) == 0
            {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            // Never deny owner/materialization/teardown SQL. Only the first
            // precise R0 read is paused, before the first probe can be polled.
            Authorization::Allow
        }));
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move {
        service
            .mutate_nal_network(database, key, fixture::mutation(), 2)
            .await
    });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    release_tx.send(()).unwrap();
    assert!(stopping && pending);
    let error = timeout(WAIT, client).await.unwrap().unwrap().unwrap_err();
    let observed_reads = reads.load(Ordering::SeqCst);
    fixture
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(error.code, "plugin_host_stopping");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert_eq!(
        error.instance_id.as_ref(),
        Some(&fixture.initial_identity.0)
    );
    // finish_failed records the actual root exit and may reattach the old
    // initialize request; the service's post-finish filter must remove it.
    assert!(error.request_id.is_none());
    assert_eq!(error.signal, Some(9));
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    assert_eq!(
        serde_json::to_value(&error).unwrap()["mutationOutcome"]["status"],
        "notStarted"
    );
    assert_eq!(observed_reads, 1);
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    fixture.assert_cleaned().await;
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.db.lock().unwrap().total_changes(), changes);
    fixture.db.shutdown().unwrap();
}
