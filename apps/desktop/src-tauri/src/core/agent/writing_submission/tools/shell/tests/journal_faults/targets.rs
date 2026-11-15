use super::*;

#[tokio::test]
async fn target_journal_failure_after_actual_patch_stays_unknown_and_stops_later_hunks() {
    let (h, access) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "target-lost";
    let target = format!("{}:file:0", key(&h, id));
    reject(&h, &target, "update");
    let call=h.call(id,"research_apply_patch",json!({"patch":format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n-Frozen manuscript\n+first changed\n*** Add File: source://writings/{}/later.tex\n+must not start\n*** End Patch",h.path(),h.fixture.manuscript)}));
    sql_unknown(&h, call.clone(), "target-progress", Some(1811)).await;
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "first changed\n"
    );
    assert!(!h.work.join("later.tex").exists());
    assert!(row(&h, &format!("{}:file:1", key(&h, id))).is_none());
    let target_before = row(&h, &target).unwrap();
    assert_eq!(target_before.0, "running");
    let outer = row(&h, &key(&h, id)).unwrap();
    assert_eq!(outer.0, "running");
    assert_eq!(outer.1["status"], "unknown");
    let detail: Value =
        serde_json::from_str(outer.1["targets"][0]["message"].as_str().unwrap()).unwrap();
    diagnostic(&h, id, &detail, "target-progress");
    assert_eq!(detail["targetRequestId"], target);
    clear(&h);
    let accesses = seal_probe(&access);
    unknown(&h, call).await;
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &target).unwrap(), target_before);
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "first changed\n"
    );
    assert!(!h.work.join("later.tex").exists());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn target_journal_begin_and_preflight_rejections_remain_definite_without_file_effects() {
    let (h, _) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "target-not-started";
    let target = format!("{}:file:0", key(&h, id));
    reject(&h, &target, "insert");
    let (error, result) = h
        .execute(h.patch(id, "Frozen manuscript", "must not apply"))
        .await;
    assert!(error);
    assert_eq!(result["targets"][0]["status"], "failed");
    assert!(row(&h, &target).is_none());
    let detail: Value =
        serde_json::from_str(result["targets"][0]["message"].as_str().unwrap()).unwrap();
    assert_eq!(detail["code"], "TEXT_PATCH_JOURNAL_UNAVAILABLE");
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    assert!(!detail.to_string().contains("private synthetic"));
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "Frozen manuscript"
    );
    clear(&h);
    let (error, result) = h
        .execute(h.patch("preflight-not-started", "missing context", "must not apply"))
        .await;
    assert!(error);
    assert_eq!(result["targets"][0]["status"], "failed");
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
