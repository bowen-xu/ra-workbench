use super::*;
use rusqlite::hooks::{Action, AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::mpsc;

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_shutdown_cancels_and_drains_the_original_provider_wait() {
    let gate = Arc::new(IndexGate::default());
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            gate: Some(Arc::clone(&gate)),
            ..Default::default()
        },
    )
    .await;
    let manager = SemanticIndexManager::default();
    let summary = start(&f, embedding, manager.clone()).await;
    tokio::time::timeout(Duration::from_secs(20), gate.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), f.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    let db = f.store.connection().unwrap();
    let state: (String, String, String, String) = db
        .query_row(
            "select j.status,j.error_code,r.status,r.error_code from embedding_outbox_jobs j join embedding_records r on r.id=j.embedding_record_id where j.id=?1",
            [&summary.job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "canceled".into(),
            "canceled".into(),
            "failed".into(),
            "canceled".into()
        )
    );
    assert!(!manager.cancel(&summary.job_id).unwrap());
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_detached_retry_keeps_the_accepted_original_job_owner() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            calls: Arc::clone(&calls),
            ..Default::default()
        },
    )
    .await;
    f.store.connection().unwrap().execute_batch("update embedding_outbox_jobs set status='failed',error_code='synthetic',finished_at='now'; update embedding_records set status='failed',error_code='synthetic'").unwrap();
    let (entered_send, entered_receive) = mpsc::channel();
    let (release_send, release_receive) = mpsc::channel();
    let inserts = Arc::new(AtomicUsize::new(0));
    {
        let inserts = Arc::clone(&inserts);
        f.store.connection().unwrap().update_hook(Some(
            move |action, _database: &str, table: &str, _row: i64| {
                if action == Action::SQLITE_INSERT && table == "embedding_outbox_jobs" {
                    inserts.fetch_add(1, Ordering::SeqCst);
                    let _ = entered_send.send(());
                    let _ = release_receive.recv_timeout(Duration::from_secs(30));
                }
            },
        ));
    }
    let (representation, _) = job_info(&f);
    let store = f.store.clone();
    let manager = SemanticIndexManager::default();
    let selected_manager = manager.clone();
    let caller = tokio::spawn(async move {
        index_commands::retry_selected(store, embedding, selected_manager, representation).await
    });
    let reached =
        tokio::task::spawn_blocking(move || entered_receive.recv_timeout(Duration::from_secs(30)))
            .await
            .unwrap();
    if reached.is_err() {
        let _ = release_send.send(());
        caller.abort();
        f.service.shutdown().await.unwrap();
        panic!("original retry insert barrier was not reached");
    }
    caller.abort();
    let disconnected = caller.await.unwrap_err().is_cancelled();
    release_send.send(()).unwrap();
    assert!(disconnected);
    let (_, job) = job_info(&f);
    let result = wait_job(&f, &job).await;
    assert_eq!(result.status, "succeeded");
    assert_eq!(result.attempt, 2);
    assert_eq!(inserts.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    f.service.shutdown().await.unwrap();
    assert!(!manager.cancel(&job).unwrap());
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_unconfirmed_retry_rollback_fences_core_before_native_start() {
    let (mut f, _, embedding) = fixture(true, IndexEmbedding::default()).await;
    f.service.shutdown().await.unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    assert!(f.service.host.lock().await.is_none());
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let (representation, _) = job_info(&f);
    let failure = index_commands::retry_selected(
        f.store.clone(),
        embedding,
        SemanticIndexManager::default(),
        representation.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure["nativeFailure"]["outcome"], "unknown");
    assert_eq!(
        failure["nativeFailure"]["rollbackError"]["phase"],
        "rollback"
    );
    assert_eq!(failure["representationId"], representation);
    assert!(f.store.connection().is_err());
    assert!(
        f.service.host.lock().await.is_none(),
        "unconfirmed rollback cannot become a Native request"
    );
    f.service.shutdown().await.unwrap();
}
