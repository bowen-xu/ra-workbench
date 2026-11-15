use super::*;

#[tokio::test]
async fn outer_progress_journal_failure_recovers_only_the_original_confirmed_target_receipt() {
    let (h, access) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "outer-progress-lost";
    reject(&h, &key(&h, id), "update");
    let call = h.patch(id, "Frozen manuscript", "progress changed");
    sql_unknown(&h, call.clone(), "call-progress", Some(1811)).await;
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "progress changed\n"
    );
    let target = format!("{}:file:0", key(&h, id));
    let target_before = row(&h, &target).unwrap();
    assert_eq!(target_before.0, "complete");
    assert_eq!(row(&h, &key(&h, id)).unwrap().0, "running");
    // The same fault still prevents reconciliation; it never allows a patch.
    let accesses = seal_probe(&access);
    sql_unknown(&h, call.clone(), "call-recovery", Some(1811)).await;
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    clear(&h);
    let output = h.tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!output.is_error);
    h.tools.result_committed(call).await.unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &target).unwrap(), target_before);
    assert_eq!(row(&h, &key(&h, id)).unwrap().0, "complete");
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn outer_terminal_journal_failure_keeps_known_effects_but_requires_original_call_reconciliation()
 {
    let (h, access) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "outer-terminal-lost";
    reject(&h, &key(&h, id), "terminal");
    let call = h.patch(id, "Frozen manuscript", "terminal changed");
    sql_unknown(&h, call.clone(), "call-terminal", Some(1811)).await;
    let outer = row(&h, &key(&h, id)).unwrap();
    assert_eq!(outer.0, "running");
    assert_eq!(outer.1["targets"][0]["status"], "applied");
    let target = format!("{}:file:0", key(&h, id));
    let target_before = row(&h, &target).unwrap();
    assert_eq!(target_before.0, "complete");
    clear(&h);
    let accesses = seal_probe(&access);
    let output = h.tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!output.is_error);
    h.tools.result_committed(call).await.unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &target).unwrap(), target_before);
    assert_eq!(row(&h, &key(&h, id)).unwrap().0, "complete");
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "terminal changed\n"
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
