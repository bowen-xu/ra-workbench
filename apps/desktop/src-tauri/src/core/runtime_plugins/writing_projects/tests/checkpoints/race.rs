use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_checkpoint_rechecks_a_concurrent_identical_write_and_returns_its_receipt() {
    use std::{
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };
    let fixture = Arc::new(Fixture::new());
    let store = store(&fixture);
    let first = save(
        &fixture,
        &store,
        CheckpointWrite {
            expected_revision: None,
            checkpoint: document(&fixture.active),
        },
    )
    .await
    .unwrap();
    let mut wanted = first.checkpoint;
    wanted.version += 1;
    wanted.content.push_str(" user input");
    let next_json = serde_json::to_string(&wanted).unwrap();
    let request = CheckpointWrite {
        expected_revision: Some(1),
        checkpoint: wanted.clone(),
    };
    let captured = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&captured);
    let seen = AtomicBool::new(false);
    let inserts = Arc::new(AtomicUsize::new(0));
    let attempted = Arc::clone(&inserts);
    let (release, waiting) = std::sync::mpsc::channel();
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "writing_document_checkpoints"
                }
            ) {
                attempted.fetch_add(1, Ordering::SeqCst);
            }
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "writing_document_checkpoints",
                    column_name: "checkpoint_json"
                }
            ) && !seen.swap(true, Ordering::SeqCst)
            {
                signal.notify_one();
                if waiting.recv_timeout(Duration::from_secs(90)).is_err() {
                    return Authorization::Deny;
                }
            }
            Authorization::Allow
        }));
    let operation_fixture = Arc::clone(&fixture);
    let operation_store = store.clone();
    let saving =
        tokio::spawn(async move { save(&operation_fixture, &operation_store, request).await });
    tokio::time::timeout(Duration::from_secs(90), captured.notified())
        .await
        .unwrap();
    let database = Arc::clone(&fixture.database);
    let id = fixture.active.clone();
    let path = wanted.relative_path.clone();
    let entered = Arc::new(tokio::sync::Notify::new());
    let entering = Arc::clone(&entered);
    let writer = tokio::task::spawn_blocking(move || {
        entering.notify_one();
        database.lock().unwrap().execute("update writing_document_checkpoints set revision=2,checkpoint_json=?3 where manuscript_id=?1 and relative_path=?2 and revision=1",rusqlite::params![id,path,next_json]).unwrap()
    });
    tokio::time::timeout(Duration::from_secs(90), entered.notified())
        .await
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(writer.await.unwrap(), 1);
    let actual = tokio::time::timeout(Duration::from_secs(90), saving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(actual.revision, 2);
    assert_eq!(actual.checkpoint, wanted);
    assert_eq!(inserts.load(Ordering::SeqCst), 0);
    assert_eq!(
        list(&fixture, &store, &fixture.active).await.unwrap(),
        vec![actual]
    );
    fixture.service.shutdown().await.unwrap();
}
