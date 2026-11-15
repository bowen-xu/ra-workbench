use super::*;

#[tokio::test]
async fn dropped_shared_file_waiter_keeps_original_mutex_until_actual_worker_finishes() {
    let h = Harness::new().await;
    assert!(!h.read("initialize-file-owner").await.0);
    let owner = h.source.workspace(&h.record).await.unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let completed = Arc::new(AtomicBool::new(false));
    let done = completed.clone();
    let host = owner.host.clone();
    let waiter = tokio::spawn(async move {
        host.with_owned_lease_mut(move |lease| {
            lease.working_directory().unwrap();
            lease.changes().unwrap();
            entered.send(()).unwrap();
            wait.recv().unwrap();
            done.store(true, Ordering::Release);
        })
        .await
    });
    started.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let tools = h.tools.clone();
    let done = completed.clone();
    let handoff = tokio::spawn(async move {
        let released = tools.release_for_finalization().await.unwrap();
        assert!(
            done.load(Ordering::Acquire),
            "handoff cannot precede the actual worker"
        );
        released
    });
    assert!(!completed.load(Ordering::Acquire));
    assert!(!handoff.is_finished());
    release.send(()).unwrap();
    assert!(handoff.await.unwrap());
    assert!(completed.load(Ordering::Acquire));
    assert!(matches!(
        h.tools
            .authorize_and_execute(h.call(
                "sealed-after-retained-worker",
                "research_read_file",
                json!({"path": h.path()}),
            ))
            .await,
        Err(RunError::ToolOutcomeUnknown | RunError::ToolOutcomeUnknownDiagnostic(_))
    ));
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
