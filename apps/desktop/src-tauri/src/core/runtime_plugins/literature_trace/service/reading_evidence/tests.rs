#![cfg(unix)]
use super::*;
use rusqlite::hooks::Action;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        mpsc,
    },
    task::Poll,
};
use tokio::time::timeout;
mod fixture;
mod selection;
mod sources;
use fixture::Fixture;
const WAIT: Duration = Duration::from_secs(8);

// Both native-selected legacy migration and provider-absent startup write Core
// data. Their accepted transactions must survive a detached caller and drain
// before shutdown closes the original SQL/log owner.
#[tokio::test]
async fn accepted_legacy_and_provider_absent_startup_drain_after_caller_abort() {
    for provider in [true, false] {
        let fixture = Fixture::new(provider).await;
        let table = if provider {
            "structured_resource_legacy_imports"
        } else {
            "structured_resources"
        };
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let expired = Arc::new(AtomicBool::new(false));
        {
            let attempts = Arc::clone(&attempts);
            let expired = Arc::clone(&expired);
            fixture.db.lock().unwrap().update_hook(Some(
                move |action, database: &str, found: &str, _rowid| {
                    if action == Action::SQLITE_INSERT
                        && database == "main"
                        && found == table
                        && attempts.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        let _ = entered_tx.send(());
                        if release_rx.recv_timeout(WAIT).is_err() {
                            expired.store(true, Ordering::SeqCst);
                        }
                    }
                },
            ));
        }
        let owner = Arc::clone(&fixture.service);
        let db = Arc::clone(&fixture.db);
        let key = fixture.key.clone();
        let client = tokio::spawn(async move {
            if provider {
                owner
                    .structured_legacy(db, Some(key), Fixture::legacy())
                    .await
                    .map(|_| ())
            } else {
                let mut draft = Fixture::startup();
                draft.catalog = Some(
                    serde_json::json!({"templates":[{"id":"synthetic-catalog","label":"Synthetic template","fields":[{"id":"question","title":"Question","question":"What was studied?"}]}]}),
                );
                owner
                    .structured_startup(db, None, draft, None)
                    .await
                    .map(|_| ())
            }
        });
        let entered = timeout(
            WAIT,
            tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
        )
        .await;
        if !matches!(entered, Ok(Ok(Ok(())))) {
            let _ = release_tx.send(());
            client.abort();
            let _ = client.await;
            let closed = timeout(WAIT, fixture.service.shutdown()).await;
            panic!("original Core transaction not reached: {entered:?}; cleanup {closed:?}");
        }
        client.abort();
        let cancelled = timeout(WAIT, client)
            .await
            .is_ok_and(|result| result.is_err_and(|error| error.is_cancelled()));
        let gate_retained = fixture.service.doi_operations.try_write().is_err();
        let host_retained = !provider || fixture.service.host.try_lock().is_err();
        let mut shutdown = Box::pin(fixture.service.shutdown());
        let pending = std::future::poll_fn(|context| {
            Poll::Ready(shutdown.as_mut().poll(context).is_pending())
        })
        .await;
        let stopping = fixture.service.stopping.load(Ordering::Acquire);
        let rejected = timeout(
            WAIT,
            fixture
                .service
                .structured_sources(Arc::clone(&fixture.db), None, "unknown".into()),
        )
        .await;
        let released = release_tx.send(()).is_ok();
        let drained = timeout(WAIT, shutdown).await;
        assert!(
            cancelled && gate_retained && host_retained,
            "accepted Core owner was released on caller abort"
        );
        assert!(
            pending && stopping,
            "shutdown failed to drain accepted Core work"
        );
        assert!(released && !expired.load(Ordering::SeqCst));
        assert_eq!(
            rejected.unwrap().err().unwrap().code,
            "plugin_host_stopping"
        );
        drained.unwrap().unwrap();
        fixture.assert_closed().await;
        if provider {
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.imported(), 1);
            let body = fixture.body();
            assert!(body.contains("Original manual body."));
            assert_eq!(fixture.cold_reopen(), (1, body));
        } else {
            assert_eq!(fixture.imported(), 0);
            let imported_catalog: bool = fixture
                .db
                .lock()
                .unwrap()
                .query_row(
                    "select exists(select 1 from structured_resource_legacy_versions where legacy_id='template:catalog:synthetic-catalog')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(imported_catalog);
            assert_eq!(fixture.cold_reopen(), (0, String::new()));
        }
    }
}

#[tokio::test]
async fn core_static_causes_and_provider_absent_errors_keep_original_codes() {
    let absent = Fixture::new(false).await;
    for result in [
        absent
            .service
            .structured_legacy(Arc::clone(&absent.db), None, Fixture::legacy())
            .await
            .map(|_| ()),
        absent
            .service
            .structured_sources(Arc::clone(&absent.db), None, "missing".into())
            .await
            .map(|_| ()),
    ] {
        let error = result.unwrap_err();
        let original = unavailable();
        assert_eq!(
            (error.code, error.message),
            (original.code, original.message)
        );
        assert!(error.instance_id.is_none() && error.request_id.is_none());
    }
    absent.service.shutdown().await.unwrap();
    absent.assert_closed().await;
    assert_eq!(absent.cold_reopen(), (0, String::new()));
    let fixture = Fixture::new(true).await;
    let error = fixture
        .service
        .structured_sources(
            Arc::clone(&fixture.db),
            Some(fixture.key.clone()),
            "missing".into(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(error.message, "找不到结构化资源");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some());
    assert!(
        error.request_id.is_none(),
        "initialize request is not a source lookup request"
    );
    fixture.assert_no_catalog_rpc();
    fixture.service.shutdown().await.unwrap();
    fixture.assert_closed().await;
    assert_eq!(fixture.cold_reopen(), (0, String::new()));
}
