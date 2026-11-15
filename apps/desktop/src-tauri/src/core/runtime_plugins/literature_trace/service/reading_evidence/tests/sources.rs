use super::*;
use rusqlite::Connection;
use serde_json::json;

fn source(db: &Connection) -> String {
    let sha = "a".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic.pdf','application/pdf',1,'now')",[&sha]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'synthetic.pdf','now')",[&sha]).unwrap();
    db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,raw_response,created_at,finished_at) values('parse','paper','attachment',?1,'fixture','success','success','{}','now','now')",[&sha]).unwrap();
    db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('snapshot','paper',?1,1,'now')",[&sha]).unwrap();
    db.execute("insert into paper_context_items(snapshot_id,context_item_id,attachment_id,content_sha256,parse_run_id,raw_ref,item_kind,page_index,regions_json,text_content,created_at) values('snapshot','P1-T1','attachment',?1,'parse','text-1','text',0,'[]','Original frozen text.','now')",[&sha]).unwrap();
    let id:String=db.query_row("select id from structured_resources where paper_id='paper' and kind='structuredReading'",[],|r|r.get(0)).unwrap();
    let content =
        json!({"fields":[{"field":"question","statements":[{"evidenceItemIds":["P1-T1"]}]}]})
            .to_string();
    db.execute(
        "update structured_resources set content_json=?2 where id=?1",
        rusqlite::params![id, content],
    )
    .unwrap();
    db.execute("insert into structured_resource_field_sources(resource_id,revision,field_id,source_snapshot_id) select id,revision,'question','snapshot' from structured_resources where id=?1",[&id]).unwrap();
    id
}

#[tokio::test]
async fn actual_selected_source_getter_uses_original_core_projection_and_safe_sql_errors() {
    let fixture = Fixture::new(true).await;
    let imported = fixture
        .service
        .structured_legacy(
            Arc::clone(&fixture.db),
            Some(fixture.key.clone()),
            Fixture::legacy(),
        )
        .await
        .unwrap();
    assert!(imported.iter().any(|r| r.kind == "structuredReading"));
    assert_eq!(fixture.imported(), 1);
    let id = source(&fixture.db.lock().unwrap());
    let get = || {
        fixture.service.structured_sources(
            Arc::clone(&fixture.db),
            Some(fixture.key.clone()),
            id.clone(),
        )
    };
    let sources = get().await.unwrap();
    assert_eq!(sources.as_array().unwrap().len(), 1);
    assert_eq!(sources[0]["quoteExact"], "Original frozen text.");
    assert_eq!(sources[0]["fieldIds"], json!(["question"]));
    assert_eq!(sources[0]["currentAnchorStatus"], "page_level");
    {
        let db = fixture.db.lock().unwrap();
        db.execute("update paper_context_items set page_index='not an integer' where context_item_id='P1-T1'",[]).unwrap();
    }
    let error = get().await.unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "无法读取冻结原文");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_none());
    let cause = error.data_error.unwrap();
    assert_eq!(
        cause.resource_id,
        "literature.reading-evidence.snapshot-rows"
    );
    assert_eq!(cause.package_key, fixture.key);
    assert!(!cause.safe_cause.is_empty());
    // Whole-result failure leaves the resource and its original transaction
    // owner available; repairing only the synthetic bad row makes it readable.
    fixture
        .db
        .lock()
        .unwrap()
        .execute("update paper_context_items set page_index=0", [])
        .unwrap();
    assert_eq!(get().await.unwrap(), sources);
    fixture.assert_no_catalog_rpc();
    let body = fixture.body();
    fixture.service.shutdown().await.unwrap();
    fixture.assert_closed().await;
    assert_eq!(fixture.cold_reopen(), (1, body));
}

#[tokio::test]
async fn source_sql_failure_during_shutdown_never_borrows_initialize_request_id() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let fixture = Fixture::new(true).await;
    fixture
        .service
        .structured_legacy(
            Arc::clone(&fixture.db),
            Some(fixture.key.clone()),
            Fixture::legacy(),
        )
        .await
        .unwrap();
    let id = source(&fixture.db.lock().unwrap());
    assert!(
        fixture
            .service
            .host
            .lock()
            .await
            .as_ref()
            .unwrap()
            .request_identity()
            .is_some()
    );
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let expired = Arc::new(AtomicBool::new(false));
    let expired_in_hook = Arc::clone(&expired);
    fixture
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "paper_context_items",
                    ..
                }
            ) {
                let _ = entered_tx.send(());
                if release_rx.recv_timeout(WAIT).is_err() {
                    expired_in_hook.store(true, Ordering::SeqCst);
                }
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let owner = Arc::clone(&fixture.service);
    let db = Arc::clone(&fixture.db);
    let key = fixture.key.clone();
    let client = tokio::spawn(async move { owner.structured_sources(db, Some(key), id).await });
    let entered = timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await;
    if !matches!(entered, Ok(Ok(Ok(())))) {
        let _ = release_tx.send(());
        let result = timeout(WAIT, client).await;
        let closed = timeout(WAIT, fixture.service.shutdown()).await;
        panic!(
            "actual source SQL barrier not reached: {entered:?}; operation {result:?}; cleanup {closed:?}"
        );
    }
    let mut shutdown = Box::pin(fixture.service.shutdown());
    let pending =
        std::future::poll_fn(|context| Poll::Ready(shutdown.as_mut().poll(context).is_pending()))
            .await;
    let stopping = fixture.service.stopping.load(Ordering::Acquire);
    let released = release_tx.send(()).is_ok();
    let result = timeout(WAIT, client).await;
    let drained = timeout(WAIT, shutdown).await;
    // Release and settle the original worker before assertions can panic.
    assert!(pending && stopping && released && !expired.load(Ordering::SeqCst));
    let error = result.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "无法读取冻结原文");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some());
    assert!(
        error.request_id.is_none(),
        "failed-host cleanup rebound the unrelated initialize frame"
    );
    let cause = error.data_error.unwrap();
    assert_eq!(
        cause.resource_id,
        "literature.reading-evidence.snapshot-rows"
    );
    assert_eq!(cause.package_key, fixture.key);
    assert!(!cause.safe_cause.is_empty());
    drained.unwrap().unwrap();
    fixture.assert_closed().await;
    fixture
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let body = fixture.body();
    assert_eq!(fixture.cold_reopen(), (1, body));
}
