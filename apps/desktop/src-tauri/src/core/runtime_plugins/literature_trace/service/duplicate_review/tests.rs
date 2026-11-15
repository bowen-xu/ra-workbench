#![cfg(unix)]
use super::*;
use crate::runtime_plugins::literature_duplicates::DecisionSqlState;
use std::sync::{Arc, atomic::Ordering};
use tokio::time::timeout;

mod fixture;
use fixture::{Fixture, Mode};

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn normal_history_refresh_waits_for_the_same_host_without_holding_the_core_database() {
    use crate::runtime_plugins::literature_duplicates::tests::fixture::PAGE;

    let mut fixture = Fixture::new(Mode::History).await;
    let observed = Arc::new(tokio::sync::Notify::new());
    Arc::get_mut(&mut fixture.service)
        .unwrap()
        .admission_observed = Some(Arc::clone(&observed));
    let held = fixture.service.host.lock().await;
    let instance = held.as_ref().unwrap().error("test", "test").instance_id;
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.data.db);
    let key = fixture.key.clone();
    // Exercise the actual production owned body, without the outer JoinHandle
    // adding a second scheduling turn after the observed admission point.
    let request = tokio::spawn(async move {
        service
            .duplicate_review_owned(database, &key, ReviewRequest::History)
            .await
    });
    timeout(WAIT, observed.notified()).await.unwrap();
    // This current-thread task can resume only after the notified body has
    // either parked on the same mutex or returned its old busy error.
    let waiting = !request.is_finished();
    let note = {
        let mut db = fixture.data.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let revision = tx
            .query_row(
                "select revision from note_pages where id=?1",
                [PAGE],
                |row| row.get(0),
            )
            .unwrap();
        let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
        document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Core Note saved while a refresh waits for its host","styles":[]}]});
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
        serde_json::to_value(crate::notes::load_document_value(&db, PAGE).unwrap()).unwrap()
    };
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
    // Always settle/reap the real child before the admission assertion. The
    // fixture accepts shutdown before its first read on the old rejecting path.
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.assert_cleaned().await;
    assert!(
        waiting,
        "normal refresh was rejected before its host was released: {result:?}"
    );
    assert!(matches!(result, Ok(ReviewResult::History(ref values)) if values.is_empty()));
    assert_eq!(fixture.steps(), ["initialize", "history", "shutdown"]);
    assert_eq!(fixture.data.note(), note);
    let data = fixture.cold();
    assert_eq!(data.note(), note);
}

fn identity(error: &NativeHostError, fixture: &Fixture) {
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    let json = serde_json::to_string(error).unwrap();
    assert!(!json.contains("Preserved synthetic duplicate Core body"));
    assert!(!json.contains("swapped-candidate"));
}

#[tokio::test]
async fn rejection_precedes_fenced_owner_and_changed_ready_retires_before_sql() {
    let fixture = Fixture::new(Mode::Rejected).await;
    fixture.data.db.lock().unwrap().require_recovery();
    assert!(fixture.data.db.lock().is_err());
    let mut request = fixture::request();
    request.decision = "DISMISS".into();
    let error = timeout(
        WAIT,
        fixture.service.review_duplicates(
            Arc::clone(&fixture.data.db),
            fixture.key.clone(),
            ReviewRequest::Decide(request),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "不支持的文献类型：DISMISS");
    assert_eq!(error.duplicate_sql, Some(DecisionSqlState::NotDispatched));
    identity(&error, &fixture);
    assert!(fixture.data.db.lock().is_err());
    assert!(fixture.service.host.lock().await.is_some());
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.steps(), ["initialize", "prepare", "shutdown"]);
    fixture.assert_cleaned().await;
    fixture.release_fenced();

    let fixture = Fixture::new(Mode::ChangedReady).await;
    let original_note = fixture.data.note();
    let changes = fixture.data.db.lock().unwrap().total_changes();
    let error = timeout(
        WAIT,
        fixture.service.review_duplicates(
            Arc::clone(&fixture.data.db),
            fixture.key.clone(),
            ReviewRequest::Decide(fixture::request()),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.duplicate_sql, Some(DecisionSqlState::NotDispatched));
    assert_eq!(error.signal, Some(9));
    identity(&error, &fixture);
    fixture.assert_cleaned().await;
    assert_eq!(fixture.steps(), ["initialize", "prepare"]);
    assert_eq!(fixture.observation(), (None, false, 0));
    assert_eq!(fixture.data.note(), original_note);
    assert_eq!(fixture.data.db.lock().unwrap().total_changes(), changes);
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    let data = fixture.cold();
    assert_eq!(fixture::observation(&data), (None, false, 0));
    assert_eq!(data.note(), original_note);
}

#[tokio::test]
async fn aborted_waiter_and_shutdown_preserve_accepted_update_finisher_and_cold_note() {
    use rusqlite::functions::FunctionFlags;
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    let fixture = Fixture::new(Mode::Applied).await;
    let original_note = fixture.data.note();
    assert!(fixture.data.db.logged());
    let attempts = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    {
        let db = fixture.data.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "pause_owned_duplicate_sql",
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
        db.execute_batch("create temp trigger pause_owned_duplicate_sql before update of decision on duplicate_candidates
            begin select pause_owned_duplicate_sql();
            insert into synthetic_service_decision_observations(value) values('one-owned-update');end;").unwrap();
    }
    let owner = Arc::clone(&fixture.service);
    let db = Arc::clone(&fixture.data.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move {
        owner
            .review_duplicates(db, key, ReviewRequest::Decide(fixture::request()))
            .await
    });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();
    client.abort();
    let cancelled = client.await.is_err_and(|cause| cause.is_cancelled());
    let retained = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    release_tx.send(()).unwrap();
    assert!(cancelled && retained && stopping && pending);
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    fixture.assert_cleaned().await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    // A healthy shutdown exchange follows the actual finisher response. The
    // cancelled waiter receives no invented success/once-only receipt.
    assert_eq!(
        fixture.steps(),
        ["initialize", "prepare", "finish", "shutdown"]
    );
    assert_eq!(fixture.observation(), (Some("dismiss".into()), true, 1));
    assert_eq!(fixture.data.note(), original_note);
    let data = fixture.cold();
    assert_eq!(
        fixture::observation(&data),
        (Some("dismiss".into()), true, 1)
    );
    assert_eq!(data.note(), original_note);
}

#[tokio::test]
async fn wrong_finisher_retains_completed_sql_count_and_never_replays_even_zero_ignore() {
    // These are two distinct fresh SQL/log fixtures, not retry attempts.
    for ignored in [false, true] {
        let fixture = Fixture::new(Mode::WrongFinish).await;
        let original_note = fixture.data.note();
        let sql = if ignored {
            "create temp trigger observe_duplicate_finish before update of decision on duplicate_candidates
             begin insert into synthetic_service_decision_observations(value) values('before-ignore');
             select raise(ignore);end;"
        } else {
            "create temp trigger observe_duplicate_finish before update of decision on duplicate_candidates
             begin insert into synthetic_service_decision_observations(value) values('before-update');end;"
        };
        fixture.data.db.lock().unwrap().execute_batch(sql).unwrap();
        let error = timeout(
            WAIT,
            fixture.service.review_duplicates(
                Arc::clone(&fixture.data.db),
                fixture.key.clone(),
                ReviewRequest::Decide(fixture::request()),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.code, "plugin_host_response_mismatch");
        assert_eq!(
            error.duplicate_sql,
            Some(DecisionSqlState::Returned {
                affected_rows: u64::from(!ignored),
                observed_autocommit: true,
            })
        );
        assert_eq!(error.signal, Some(9));
        identity(&error, &fixture);
        fixture.assert_cleaned().await;
        assert_eq!(fixture.steps(), ["initialize", "prepare", "finish"]);
        let expected = if ignored {
            (None, false, 1)
        } else {
            (Some("dismiss".into()), true, 1)
        };
        assert_eq!(fixture.observation(), expected);
        assert_eq!(fixture.data.note(), original_note);
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        let data = fixture.cold();
        assert_eq!(fixture::observation(&data), expected);
        assert_eq!(data.note(), original_note);
    }
}

#[tokio::test]
async fn candidate_projection_checks_every_field_even_when_ids_order_and_bits_match() {
    let fixture = Fixture::new(Mode::TamperedCandidates).await;
    let original_note = fixture.data.note();
    let changes = fixture.data.db.lock().unwrap().total_changes();
    let error = timeout(
        WAIT,
        fixture.service.review_duplicates(
            Arc::clone(&fixture.data.db),
            fixture.key.clone(),
            ReviewRequest::Candidates,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.signal, Some(9));
    assert!(error.duplicate_sql.is_none());
    identity(&error, &fixture);
    fixture.assert_cleaned().await;
    assert_eq!(fixture.steps(), ["initialize", "candidates"]);
    assert_eq!(fixture.observation(), (None, false, 0));
    assert_eq!(fixture.data.note(), original_note);
    assert_eq!(fixture.data.db.lock().unwrap().total_changes(), changes);
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    let data = fixture.cold();
    assert_eq!(fixture::observation(&data), (None, false, 0));
    assert_eq!(data.note(), original_note);
}
