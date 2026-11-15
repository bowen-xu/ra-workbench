use super::*;

fn fail_lookup(h: &Harness, key: &str) {
    let db = h.fixture.store.connection().unwrap();
    // A real SQLite SELECT fault, limited to one original key. The view's
    // update trigger keeps the original durable table/results intact.
    db.execute_batch("alter table agent_text_patch_calls rename to held_patch_calls;
        create temp table patch_lookup_fault(request_id text primary key);
        create temp view agent_text_patch_calls as select p.conversation_id,p.request_id,p.input_sha256,p.state,
        case when exists(select 1 from patch_lookup_fault f where f.request_id=p.request_id)
             then json('private synthetic malformed json') else p.results_json end as results_json
        from main.held_patch_calls p;
        create temp trigger patch_lookup_update instead of update on agent_text_patch_calls
        begin update held_patch_calls set input_sha256=new.input_sha256,state=new.state,results_json=new.results_json
              where conversation_id=old.conversation_id and request_id=old.request_id;end;").unwrap();
    db.execute(
        "insert into patch_lookup_fault(request_id) values(?1)",
        [key],
    )
    .unwrap();
}
fn restore_lookup(h: &Harness) {
    h.fixture.store.connection().unwrap().execute_batch("drop trigger patch_lookup_update;drop view agent_text_patch_calls;drop table patch_lookup_fault;alter table held_patch_calls rename to agent_text_patch_calls;").unwrap();
}

#[tokio::test]
async fn original_call_lookup_failure_is_unknown_until_receipts_can_be_read_again() {
    let (h, access) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "original-read-lost";
    let original = key(&h, id);
    let target = format!("{original}:file:0");
    reject(&h, &original, "update");
    let call = h.patch(id, "Frozen manuscript", "lookup changed");
    sql_unknown(&h, call.clone(), "call-progress", Some(1811)).await;
    let target_before = row(&h, &target).unwrap();
    assert_eq!(target_before.0, "complete");
    clear(&h);
    let accesses = seal_probe(&access);
    fail_lookup(&h, &original);
    sql_unknown(
        &h,
        call.clone(),
        "journal-begin-read",
        Some(rusqlite::ffi::SQLITE_ERROR),
    )
    .await;
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &target).unwrap(), target_before);
    assert_eq!(row(&h, &original).unwrap().0, "running");
    restore_lookup(&h);
    let output = h.tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!output.is_error);
    h.tools.result_committed(call).await.unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(row(&h, &original).unwrap().0, "complete");
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "lookup changed\n"
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_target_from_failed_query_rechecks_the_original_complete_receipt_after_recovery() {
    let (h, access) = observed().await;
    assert!(!h.read("read").await.0);
    let id = "target-query-lost";
    let original = key(&h, id);
    let target = format!("{original}:file:0");
    reject(&h, &original, "update");
    let call = h.patch(id, "Frozen manuscript", "query changed");
    sql_unknown(&h, call.clone(), "call-progress", Some(1811)).await;
    let target_before = row(&h, &target).unwrap();
    assert_eq!(target_before.0, "complete");
    clear(&h);
    let accesses = seal_probe(&access);
    fail_lookup(&h, &target);
    sql_unknown(
        &h,
        call.clone(),
        "target-query",
        Some(rusqlite::ffi::SQLITE_ERROR),
    )
    .await;
    let failed = row(&h, &original).unwrap();
    assert_eq!(failed.0, "running");
    assert_eq!(failed.1["targets"][0]["status"], "unknown");
    let detail: Value =
        serde_json::from_str(failed.1["targets"][0]["message"].as_str().unwrap()).unwrap();
    assert_eq!(detail["phase"], "target-query");
    assert_eq!(detail["sqliteExtendedCode"], rusqlite::ffi::SQLITE_ERROR);
    assert!(!detail.to_string().contains("private synthetic"));
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    restore_lookup(&h);
    let output = h.tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!output.is_error);
    h.tools.result_committed(call).await.unwrap();
    let known = row(&h, &original).unwrap();
    assert_eq!(known.0, "complete");
    assert_eq!(known.1["targets"][0]["status"], "applied");
    assert_eq!(row(&h, &target).unwrap(), target_before);
    assert_eq!(access.calls.load(Ordering::SeqCst), accesses);
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "query changed\n"
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
