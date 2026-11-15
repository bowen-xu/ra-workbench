//! Source changes during a Native wait are rejected before selected-evidence writes.
use super::*;
#[tokio::test]
async fn selection_evidence_wait_releases_guard_and_rejects_changed_context_before_publication() {
    let (mut f, run, outputs) = pages::fixture("reference_extraction");
    f.configure(&outputs, Some(0), false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call =
        tokio::spawn(async move { owner.persist_selection_task_evidence(store, key, run).await });
    assert!(f.entered(0).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update source_anchors set quote_prefix='Changed context.' where id='reference-anchor'",
            [],
        )
        .unwrap();
    });
    let available = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let error = tokio::time::timeout(WAIT, call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(available.is_ok(), "Native wait holds no Core DB guard");
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected source-change diagnostic")
    };
    assert_eq!(failure.code, "plugin_host_source_changed");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert_eq!(f.store.connection().unwrap().query_row("select (select count(*) from ai_evidence_bundles)+(select count(*) from ai_evidence_items)",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    f.service.shutdown().await.unwrap();
}
