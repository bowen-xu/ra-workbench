#![cfg(unix)]
use super::*;
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn aborted_bootstrap_waiter_keeps_t1_owned_and_shutdown_waits_for_one_cold_reopenable_commit()
{
    use crate::data_interface::Database;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::Poll,
    };

    let fixture = Fixture::two_phases().await;
    assert!(fixture.db.logged());
    assert_eq!(fixture.snapshot().0, (0, 0, 0));
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_bootstrap_flag_barrier",
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
        // Only the original first flag write in T1 reaches this connection-local hook.
        db.execute_batch("create temp trigger test_bootstrap_flag_pause before update of legacy_imported on nal_state begin select test_bootstrap_flag_barrier();end;").unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move { service.read_nal_bootstrap(database, key).await });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT));
    timeout(WAIT, entered).await.unwrap().unwrap().unwrap();

    // A real SQL hook proves acceptance. Dropping only the external waiter must
    // not drop the already-owned SQL worker or its original host mutex.
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
        "aborted waiter released the owned bootstrap"
    );
    assert!(
        stopping && pending,
        "shutdown completed before accepted T1 was released"
    );
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    fixture.assert_cleaned().await;
    let committed = fixture.snapshot();
    assert_eq!(committed.0, (1, 1, 2));
    assert!(committed.1.iter().all(|count| *count == 0));
    assert_eq!(Arc::strong_count(&fixture.db), 1);
    assert_eq!(Arc::strong_count(&fixture.service), 1);

    // The cancelled waiter supplies no success receipt. Check actual durable
    // flags/state after releasing every old Core SQL/log and service owner.
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
        assert_eq!(fixture::flags(&db), committed.0);
        assert_eq!(fixture::domain_counts(&db), committed.1);
    }
    reopened.shutdown().unwrap();
}

#[tokio::test]
async fn a_wrong_first_operation_retires_the_exact_host_and_inputs_without_persistent_writes() {
    let fixture = Fixture::wrong_first_operation().await;
    let before = fixture.snapshot();
    let error = timeout(
        WAIT,
        fixture
            .service
            .read_nal_bootstrap(Arc::clone(&fixture.db), fixture.key.clone()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.exit_code.is_some() || error.signal.is_some());
    assert_eq!(error.bootstrap_outcome, Some(BootstrapOutcome::NotStarted));
    assert!(
        error.cleanup_error.is_none(),
        "safe native cleanup diagnostic: {error:?}"
    );
    fixture.assert_cleaned().await;
    assert_eq!(
        fixture.snapshot(),
        before,
        "R0 response rejection caused a durable write"
    );
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(!diagnostic.contains(fixture.data.path().to_str().unwrap()));
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}

#[test]
fn unknown_bootstrap_error_retains_safe_problem_and_observation_without_claiming_commit_or_replay()
{
    use crate::runtime_plugins::{
        literature_nal_bootstrap::{
            BootstrapCause, BootstrapFailure, BootstrapProblem, FailureContext,
        },
        native::SafeDatabaseError,
    };
    use research_workbench_native_sdk::nal_bootstrap::BootstrapPhaseV1;
    let error = errors::failure(BootstrapFailure {
        problem: BootstrapProblem {
            phase: BootstrapPhaseV1::ImportFlag,
            context: FailureContext::Network,
            cause: BootstrapCause::InvalidBinding("fixed_safe_binding_problem"),
        },
        outcome: BootstrapOutcome::Unknown {
            observed_revision: Some(2),
        },
        rollback_error: Some(Box::new(SafeDatabaseError::invariant(
            "bootstrap_transaction_unsettled",
            "rollback",
        ))),
    });
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "storage_error");
    assert_eq!(
        value["message"],
        "关系网络提交或回滚结果未知，禁止自动重试。"
    );
    assert_eq!(
        value["bootstrapOutcome"],
        serde_json::json!({"status":"unknown","observedRevision":2})
    );
    assert_eq!(value["bootstrapProblem"]["phase"], "importFlag");
    assert_eq!(value["bootstrapProblem"]["cause"]["kind"], "invalidBinding");
    assert_eq!(
        value["bootstrapProblem"]["cause"]["diagnostic"],
        "fixed_safe_binding_problem"
    );
    assert_eq!(
        value["bootstrapRollbackError"]["safeCause"],
        "bootstrap_transaction_unsettled"
    );
    assert!(value["bootstrapOutcome"].get("revision").is_none());
    assert!(value.get("saveOutcome").is_none());
}

#[tokio::test]
async fn a_typed_maximum_statement_count_is_rejected_before_identity_allocation_or_t1() {
    // Run only with the production physical-input guard registered. A tiny,
    // valid DTO must not let its scalar count trigger an unbounded allocation.
    let fixture = Fixture::impossible_count().await;
    let before = fixture.snapshot();
    let error = timeout(
        WAIT,
        fixture
            .service
            .read_nal_bootstrap(Arc::clone(&fixture.db), fixture.key.clone()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.bootstrap_outcome, Some(BootstrapOutcome::NotStarted));
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
        "impossible count entered a write path"
    );
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.db.shutdown().unwrap();
}
