use super::*;

#[tokio::test]
async fn ignored_target_or_outer_terminal_update_never_fakes_a_committed_receipt() {
    for target_fault in [false, true] {
        let (h, access) = observed().await;
        assert!(!h.read("read").await.0);
        let id = "ignored-terminal";
        let original = key(&h, id);
        let target = format!("{original}:file:0");
        reject(
            &h,
            if target_fault { &target } else { &original },
            "ignore-terminal",
        );
        let call = h.patch(id, "Frozen manuscript", "zero changed");
        let detail = sql_unknown(
            &h,
            call.clone(),
            if target_fault {
                "target-terminal"
            } else {
                "call-terminal"
            },
            None,
        )
        .await;
        assert_eq!(detail["safeCause"], "journal_receipt_missing");
        assert_eq!(
            std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
            "zero changed\n"
        );
        assert_eq!(row(&h, &original).unwrap().0, "running");
        if target_fault {
            let outer = row(&h, &original).unwrap();
            let detail: Value =
                serde_json::from_str(outer.1["targets"][0]["message"].as_str().unwrap()).unwrap();
            assert_eq!(detail["safeCause"], "journal_receipt_missing");
            assert!(detail["sqliteCode"].is_null());
            assert!(detail["sqliteExtendedCode"].is_null());
            assert_eq!(row(&h, &target).unwrap().0, "running");
        } else {
            assert_eq!(row(&h, &target).unwrap().0, "complete");
        }
        clear(&h);
        let accesses = seal_probe(&access);
        if target_fault {
            unknown(&h, call).await;
            assert_eq!(row(&h, &original).unwrap().0, "running");
        } else {
            let output = h.tools.authorize_and_execute(call.clone()).await.unwrap();
            assert!(!output.is_error);
            h.tools.result_committed(call).await.unwrap();
            assert_eq!(row(&h, &original).unwrap().0, "complete");
        }
        assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
        assert_eq!(h.original(), "Frozen manuscript");
        h.sessions.processes().shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn ignored_confirmed_absent_target_begin_is_a_definite_rejection_before_any_patch() {
    let (h, _) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "ignored-begin";
    let target = format!("{}:file:0", key(&h, id));
    reject(&h, &target, "ignore-insert");
    let (error, value) = h
        .execute(h.patch(id, "Frozen manuscript", "must not apply"))
        .await;
    assert!(error);
    assert_eq!(value["targets"][0]["status"], "failed");
    assert!(row(&h, &target).is_none());
    let detail: Value =
        serde_json::from_str(value["targets"][0]["message"].as_str().unwrap()).unwrap();
    assert_eq!(detail["code"], "TEXT_PATCH_JOURNAL_UNAVAILABLE");
    assert_eq!(detail["safeCause"], "journal_receipt_missing");
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "Frozen manuscript"
    );
    assert_eq!(h.original(), "Frozen manuscript");
    clear(&h);
    h.sessions.processes().shutdown().await.unwrap();
}
