#![cfg(unix)]
use super::*;
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn aborted_query_client_keeps_t1_owned_until_shutdown_and_one_cold_reopenable_commit() {
    use crate::data_interface::Database;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    let fixture = Fixture::prepare_save().await;
    assert!(fixture.db.logged());
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_nal_query_save_barrier",
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
        db.execute_batch("create temp trigger test_nal_query_save_pause before update on nal_query_collections begin select test_nal_query_save_barrier();end;").unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let content = fixture.content.clone();
    let client =
        tokio::spawn(async move { service.save_nal_queries(database, key, content, 0).await });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();

    // The actual SQL trigger is paused inside T1. Only the caller's wait is
    // aborted; the production owned task must retain both SQL and host owners.
    client.abort();
    let cancelled = client.await.is_err_and(|cause| cause.is_cancelled());
    let retained_host_owner = fixture.service.host.try_lock().is_err();
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    // Release before assertions so a failed owner check cannot strand a SQL worker.
    release_tx.send(()).unwrap();
    assert!(
        cancelled && retained_host_owner,
        "client abort released the Core-owned query save"
    );
    assert!(
        stopping && pending,
        "shutdown did not wait for the accepted T1"
    );
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    fixture.assert_cleaned().await;
    let stored = fixture.snapshot().0.decode().unwrap();
    assert_eq!(
        stored,
        Stored {
            revision: 1,
            content: fixture.content.clone()
        }
    );
    assert_eq!(Arc::strong_count(&fixture.db), 1);
    assert_eq!(Arc::strong_count(&fixture.service), 1);

    // No public result is inferred from the aborted waiter. The durable row is
    // checked after all old Core owners are released and the SQL/log pair reopens.
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
        let db = reopened.lock().unwrap();
        let actual = db.query_row(
            "select revision,definitions,cache,legacy_imported from nal_query_collections where id=1",
            [], |row| Ok(research_workbench_native_sdk::nal_query::StoredRow {
                revision: row.get(0)?, definitions_json: row.get(1)?,
                cache_json: row.get(2)?, legacy_imported: row.get(3)?,
            }),
        ).unwrap().decode().unwrap();
        assert_eq!(actual, stored);
        assert_eq!(
            db.query_row("select count(*) from nal_query_collections", [], |row| row
                .get::<_, u64>(
                0
            ))
            .unwrap(),
            1
        );
    }
    reopened.shutdown().unwrap();
}

#[tokio::test]
async fn unequal_raw_query_row_in_a_valid_reply_retires_host_without_any_database_write() {
    let fixture = Fixture::wrong_read().await;
    let before = fixture.snapshot();
    let error = timeout(
        WAIT,
        fixture
            .service
            .read_nal_queries(Arc::clone(&fixture.db), fixture.key.clone()),
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
    fixture.assert_cleaned().await;
    assert_eq!(
        fixture.snapshot(),
        before,
        "read projection performed a database write"
    );
    let diagnostic = serde_json::to_string(&error).unwrap();
    for private in [
        " [] ",
        "definitionsJson",
        fixture.data.path().to_str().unwrap(),
    ] {
        assert!(!diagnostic.contains(private));
    }
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}

#[test]
fn unknown_query_failure_json_keeps_an_observation_without_exposing_a_cas_retry_message() {
    use crate::runtime_plugins::literature_nal_query::{QueryCause, QueryFailure};
    let error = errors::failure(QueryFailure {
        cause: QueryCause::Conflict,
        outcome: QueryOutcome::Unknown {
            observed_revision: Some(7),
        },
        rollback_error: None,
    });
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "storage_error");
    assert_eq!(
        value["queryOutcome"],
        serde_json::json!({"status":"unknown","observedRevision":7})
    );
    assert_eq!(value["queryErrorKind"], "nal_query_cas_conflict");
    assert!(
        !value["message"]
            .as_str()
            .unwrap()
            .contains("查询集合已变化")
    );
    assert!(value.get("saveOutcome").is_none());
}
