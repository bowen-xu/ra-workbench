use super::*;
use crate::data_interface::guarded_transaction;
use research_workbench_native_sdk::catalog::CatalogOutput;
use rusqlite::hooks::Action;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Poll,
};

#[tokio::test]
async fn a_failed_second_actual_exchange_keeps_its_own_identity_and_never_invents_metadata_proof() {
    for wrong_echo in [false, true] {
        let mut fixture = Fixture::new();
        let good = fixture.response();
        let outputs = if wrong_echo {
            let mut wrong = good.clone();
            let CatalogOutput::ContextMetadata { outcome } = &mut wrong else {
                panic!("metadata output")
            };
            outcome.input.cbor_base64 = "different-second-input".into();
            vec![good, wrong]
        } else {
            // The shell exits at its existing finite unmatched-output branch.
            vec![good]
        };
        fixture.configure(&outputs, None, false);
        let first = fixture.execute(fixture.metadata_read()).await;
        assert!(!first.is_error, "{}", first.content);
        let request = fixture.metadata_read();
        let failed = fixture.execute(request.clone()).await;
        assert!(failed.is_error);
        let value: Value = serde_json::from_str(&failed.content).unwrap();
        let requests = fixture
            .lifecycle()
            .lines()
            .filter_map(|line| line.strip_prefix("request ").map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0], requests[1]);
        assert_eq!(value["requestId"], fixture.record.request_id.to_string());
        assert_eq!(value["turnId"], fixture.record.turn_id.to_string());
        assert_eq!(value["callId"], request.call_id);
        assert_eq!(value["nativeFailure"]["packageKey"], fixture.key);
        assert_eq!(value["nativeFailure"]["nativeRequestId"], requests[1]);
        assert!(
            value["nativeFailure"]["instanceId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        if wrong_echo {
            assert_eq!(
                value["nativeFailure"]["code"],
                "plugin_host_response_mismatch"
            );
        }
        assert_eq!(fixture.proofs(), 0);
        let _ = fixture.service.shutdown().await;
    }
}

#[tokio::test]
async fn cancelled_tool_waiter_keeps_owned_prefix_and_shutdown_waits_for_original_note_proof() {
    let mut fixture = Fixture::new();
    fixture.mixed(false);
    let response = fixture.response();
    fixture.configure(&[response], Some(0), false);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = AtomicBool::new(true);
    fixture.store.connection().unwrap().update_hook(Some(
        move |action, database: &str, table: &str, _rowid| {
            if action == Action::SQLITE_INSERT
                && database == "main"
                && table == "agent_text_read_scopes"
                && first.swap(false, Ordering::AcqRel)
            {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(WAIT);
            }
        },
    ));
    let caller = fixture.spawn(fixture.search());
    let entered = fixture.entered(0).await;
    caller.abort();
    let cancelled = caller.await.is_err_and(|error| error.is_cancelled());
    let store = fixture.store.clone();
    let independent = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            let id = crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Independent Core note during metadata await".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                None,
            )
            .unwrap();
            tx.commit().unwrap();
            id
        }),
    )
    .await;
    fixture.release();
    let proof_entered = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await
    .is_ok_and(|join| join.is_ok_and(|received| received.is_ok()));
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let _ = release_tx.send(());
    tokio::time::timeout(WAIT, shutdown).await.unwrap().unwrap();
    assert!(entered && cancelled && proof_entered && pending);
    let note = independent.unwrap().unwrap();
    assert_eq!(
        fixture
            .store
            .connection()
            .unwrap()
            .query_row("select title from note_pages where id=?1", [note], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap(),
        "Independent Core note during metadata await"
    );
    assert_eq!(fixture.proofs(), 2);
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
    assert!(fixture.service.host.lock().await.is_none());
}

#[tokio::test]
async fn shutdown_during_an_actual_metadata_exchange_retains_that_new_request_and_drains_owner() {
    let mut fixture = Fixture::new();
    let response = fixture.response();
    fixture.configure(&[response], Some(0), false);
    let request = fixture.metadata_read();
    let caller = fixture.spawn(request);
    let entered = fixture.entered(0).await;
    let actual = fixture
        .lifecycle()
        .lines()
        .find_map(|line| line.strip_prefix("request ").map(str::to_owned));
    let drained = tokio::time::timeout(WAIT, fixture.service.shutdown()).await;
    fixture.release();
    let output = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    drained.unwrap().unwrap();
    assert!(entered && output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(value["nativeFailure"]["nativeRequestId"], actual.unwrap());
    assert_eq!(value["nativeFailure"]["packageKey"], fixture.key);
    assert_eq!(fixture.proofs(), 0);
    assert!(fixture.service.host.lock().await.is_none());
}

#[tokio::test]
async fn unsettled_original_connection_fences_metadata_attempt_before_any_host_admission() {
    let mut fixture = Fixture::new();
    fixture.configure(&[], None, false);
    let executor = fixture.executor();
    fixture
        .store
        .connection()
        .unwrap()
        .execute_batch("begin deferred;")
        .unwrap();
    let output = executor
        .authorize_and_execute(fixture.metadata_read())
        .await
        .unwrap();
    assert!(output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(
        value["nativeFailure"]["code"],
        "context_metadata_worker_unsettled"
    );
    assert_eq!(value["nativeFailure"]["outcome"], "unknown");
    assert!(value["nativeFailure"]["nativeRequestId"].is_null());
    assert!(fixture.store.connection().is_err());
    assert!(fixture.lifecycle().is_empty());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn original_guard_panic_blocks_later_actual_tool_admission_without_starting_a_host() {
    let mut fixture = Fixture::new();
    fixture.configure(&[], None, false);
    let executor = fixture.executor();
    let store = fixture.store.clone();
    let panicked = tokio::task::spawn_blocking(move || {
        let mut db = store.connection().unwrap();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _: Result<(), ()> = guarded_transaction::run_mut(&mut db, |db| {
                db.execute_batch("begin deferred;").unwrap();
                panic!("controlled original guarded-transaction unwind");
            });
        }));
        result.is_err() && !db.is_autocommit()
    })
    .await
    .unwrap();
    assert!(panicked);
    assert!(fixture.store.connection().is_err());
    let output = executor
        .authorize_and_execute(fixture.metadata_read())
        .await
        .unwrap();
    assert!(output.is_error);
    assert!(fixture.lifecycle().is_empty());
    fixture.service.shutdown().await.unwrap();
}
