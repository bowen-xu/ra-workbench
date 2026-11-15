use super::*;
async fn proposed() -> Fixture {
    let f = fixture(true);
    let path = path(f.page);
    read(&f, &path).await;
    f.tools
        .apply("proposal", &patch(&path, "old", "new"))
        .await
        .unwrap();
    f
}
#[tokio::test]
async fn decision_receipts_and_definitive_failures_do_not_reexecute() {
    let f = proposed().await;
    let shown = review(&f);
    let request = decision(&f, &shown, "accept");
    let first = decision_requests::run(f.tools.store.clone(), request.clone())
        .await
        .unwrap_or_else(|error| panic!("{}", error.message));
    let repeated = decision_requests::run(f.tools.store.clone(), request)
        .await
        .unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(repeated).unwrap()
    );
    let f = proposed().await;
    let shown = review(&f);
    let request = decision(&f, &shown, "accept");
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set revision=revision+1 where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    let error = decision_requests::run(f.tools.store.clone(), request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, "failed");
    let error = decision_requests::run(f.tools.store.clone(), request)
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, "failed");
    assert!(
        decision_requests::run(f.tools.store.clone(), decision(&f, &shown, "reject"))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn live_unknown_requests_are_not_replayed_and_dead_process_requests_resume_by_original_identity()
 {
    let f = proposed().await;
    let shown = review(&f);
    let request = decision(&f, &shown, "accept");
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&request).unwrap())
    );
    f.tools.store.connection().unwrap().execute("insert into agent_note_decision_requests(request_id,conversation_id,input_sha256,owner_boot,owner_pid,state) values(?1,?2,?3,?4,?5,'running')",params![request.request_id.to_string(),request.conversation_id.to_string(),hash,f.tools.store.text_patch_boot().to_string(),std::process::id()]).unwrap();
    assert_eq!(
        decision_requests::run(f.tools.store.clone(), request.clone())
            .await
            .err()
            .unwrap()
            .status,
        "unknown"
    );
    f.tools.store.connection().unwrap().execute("update agent_note_decision_requests set owner_boot='previous-process',owner_pid=2147483647 where request_id=?1",[request.request_id.to_string()]).unwrap();
    let receipt = decision_requests::run(f.tools.store.clone(), request.clone())
        .await
        .unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(receipt.request_id, request.request_id);
}
#[tokio::test]
async fn conversation_deletion_releases_candidates_receipts_and_read_caches() {
    let f = proposed().await;
    let db = f.tools.store.connection().unwrap();
    db.execute(
        "delete from agent_conversations where id=?1",
        [f.tools.record.conversation_id.to_string()],
    )
    .unwrap();
    for table in [
        "agent_note_patch_candidates",
        "agent_text_read_scopes",
        "agent_text_patch_calls",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
