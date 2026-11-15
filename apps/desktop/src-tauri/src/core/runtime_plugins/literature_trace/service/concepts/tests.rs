#![cfg(unix)]
use super::*;
use rusqlite::hooks::Action;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
};
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn aborted_concept_caller_retains_accepted_sql_until_shutdown_and_cold_reopen() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    assert!(fixture.db.logged());
    assert!(before.rows["concepts"].is_empty());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let barrier_timed_out = Arc::new(AtomicBool::new(false));
    {
        let attempts = Arc::clone(&attempts);
        let timed_out = Arc::clone(&barrier_timed_out);
        fixture.db.lock().unwrap().update_hook(Some(
            move |action, database: &str, table: &str, _rowid| {
                if action == Action::SQLITE_INSERT && database == "main" && table == "concepts" {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_tx.send(());
                    if release_rx.recv_timeout(WAIT).is_err() {
                        timed_out.store(true, Ordering::SeqCst);
                    }
                }
            },
        ));
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client =
        tokio::spawn(async move { service.concepts(database, key, fixture::request()).await });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    let entered = timeout(WAIT, entered).await;
    if !matches!(entered, Ok(Ok(Ok(())))) {
        // Release before any panic; the callback also has a bounded wait.
        let _ = release_tx.send(());
        client.abort();
        let _ = client.await;
        let cleanup = timeout(WAIT, fixture.service.shutdown()).await;
        panic!("concept SQL barrier was not reached: {entered:?}; cleanup: {cleanup:?}");
    }
    client.abort();
    let cancelled = timeout(WAIT, client)
        .await
        .is_ok_and(|result| result.is_err_and(|cause| cause.is_cancelled()));
    let retained_host = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    let rejected = timeout(
        WAIT,
        fixture.service.concepts(
            Arc::clone(&fixture.db),
            fixture.key.clone(),
            ConceptRequest::Search {
                query: String::new(),
            },
        ),
    )
    .await;
    // Do not assert while the real accepted transaction is held.
    let released = release_tx.send(()).is_ok();
    let drained = timeout(WAIT, shutdown).await;
    assert!(
        cancelled && retained_host,
        "caller abort released accepted SQL"
    );
    assert!(stopping && pending, "shutdown did not wait for concept SQL");
    assert!(released && !barrier_timed_out.load(Ordering::SeqCst));
    let rejection = rejected.unwrap().err().expect("new work after stop");
    assert_eq!(rejection.code, "plugin_host_stopping");
    drained.unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    fixture.assert_cleaned().await;
    let committed = fixture.snapshot();
    for (table, expected) in [
        ("concepts", 1),
        ("terms", 1),
        ("links", 1),
        ("revisions", 2),
        ("representations", 1),
        ("records", 0),
        ("jobs", 0),
    ] {
        assert_eq!(committed.rows[table].len(), expected, "{table}");
    }
    assert_eq!(committed.document, before.document);
    // No success receipt survived the aborted caller. Reopen the actual pair,
    // without replay or replacement identities, after every old owner closes.
    assert_eq!(fixture.cold_snapshot(), committed);
}
