use super::*;

#[tokio::test]
async fn actual_compile_cancel_reaps_process_and_cleans_output_before_returning_cancelled() {
    let h = Harness::new().await;
    let executable = h.compiler.toolchain.as_ref().unwrap().tools[0]
        .path
        .as_ref()
        .unwrap();
    std::fs::write(
        executable,
        b"#!/bin/sh\nread value\nprintf should-not-reach\n",
    )
    .unwrap();
    let compiler = h.compiler.clone();
    let call = h.compile_call("actual-cancel");
    let work = tokio::spawn(async move { compiler.authorize_and_execute(call).await });
    let marker = h.work.parent().unwrap().join("execution.json");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    h.sessions
        .processes()
        .stop_turn(h.record.conversation_id, h.record.turn_id)
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(output.is_error);
    let result: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["retained"], false);
    assert!(!marker.exists());
    assert!(h.outputs().is_empty());
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn dropped_compile_waiter_retains_owner_through_artifact_snapshot_and_cleanup_before_handoff()
{
    let mut h = Harness::new().await;
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let entered = std::sync::Mutex::new(Some(entered));
    let wait = std::sync::Mutex::new(wait);
    h.compiler.before_output = Some(Arc::new(move || {
        entered.lock().unwrap().take().unwrap().send(()).unwrap();
        wait.lock().unwrap().recv().unwrap();
    }));
    let compiler = h.compiler.clone();
    let call = h.compile_call("retained-compile");
    let waiter = tokio::spawn(async move { compiler.authorize_and_execute(call).await });
    tokio::time::timeout(Duration::from_secs(30), started)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        h.outputs().len(),
        1,
        "actual artifacts still await snapshot/cleanup"
    );
    assert!(
        !h.work.parent().unwrap().join("execution.json").exists(),
        "actual process already drained; mutex still owns postprocessing"
    );
    waiter.abort();
    assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
    let tools = h.tools.clone();
    let handoff = tokio::spawn(async move { tools.release_for_finalization().await });
    tokio::task::yield_now().await;
    assert!(!handoff.is_finished());
    release.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(30), handoff)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    assert!(h.outputs().is_empty());
    let cache = h
        .fixture
        .store
        .writing_cache_directory()
        .join("agent-compile-results");
    let retained: Vec<_> = std::fs::read_dir(cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        std::fs::read_to_string(retained[0].join("source/main.tex")).unwrap(),
        "Frozen manuscript"
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(retained[0].join("metadata.json")).unwrap()).unwrap();
    assert_eq!(
        metadata["identity"]["requestId"],
        h.record.request_id.to_string()
    );
    assert_eq!(
        metadata["identity"]["contextSnapshotId"],
        h.record.context_snapshot_id.to_string()
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
