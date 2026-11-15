use super::*;

#[tokio::test]
async fn version_one_actual_patch_with_failed_terminal_receipt_keeps_original_unknown_and_never_repatches()
 {
    let (h, access) = observed().await;
    h.fixture.store.connection().unwrap().execute("update agent_text_tool_contracts set version=1 where conversation_id=?1 and request_id=?2",params![h.record.conversation_id.to_string(),h.record.request_id.to_string()]).unwrap();
    let id = "legacy-terminal-lost";
    reject(&h, &key(&h, id), "terminal");
    let call=h.call(id,"research_apply_patch",json!({"operations":[{"path":"main.tex","before":"Frozen manuscript","after":"legacy changed"}]}));
    sql_unknown(&h, call.clone(), "legacy-terminal", Some(1811)).await;
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "legacy changed"
    );
    let before = row(&h, &key(&h, id)).unwrap();
    assert_eq!(before.0, "running");
    assert_eq!(before.1["status"], "unknown");
    diagnostic(&h, id, &before.1["storageFailure"], "legacy-terminal");
    clear(&h);
    let accesses = seal_probe(&access);
    unknown(&h, call).await;
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &key(&h, id)).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "legacy changed"
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
