//! New preview consumer on the original finite host/fault fixture.
use super::*;
use crate::{
    ai::paper_reading_contracts::PaperContextRequest,
    runtime_plugins::literature_context::preview::PreviewResources,
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    paper_context_wire::{
        PaperContextDecisionV1 as Decision, PaperContextInputV1 as Input, PaperContextOutcomeV1,
        encode_payload,
    },
};
pub(super) fn prepare(fixture: &Fixture) -> (PaperContextRequest, Vec<CatalogOutput>) {
    let p = fixture.prepared().projection;
    let catalog = &fixture.service.catalog;
    let package = catalog.discover().unwrap().packages.remove(0);
    let resources = PaperContextResources::load(catalog, &package).unwrap();
    let mut db = fixture.store.connection().unwrap();
    let captured = resources.capture_snapshot(&db, &p).unwrap();
    resources
        .apply_snapshot(&mut db, "paper", &p, &captured, None)
        .unwrap();
    let request = PaperContextRequest {
        paper_id: "paper".into(),
        snapshot_id: p.snapshot_id,
        expected_context_hash: p.context_hash,
    };
    let preview = PreviewResources::load(catalog, &package).unwrap();
    let outputs = [
        preview.header(&db, &request).unwrap(),
        preview.preflight(&db, &request).unwrap(),
        preview.snapshot(&db, &request).unwrap(),
    ]
    .into_iter()
    .map(|capture| {
        let decision = research_literature_library_native::paper_context_preview::evaluate(
            capture.input.clone(),
        );
        CatalogOutput::PaperContext {
            outcome: Box::new(PaperContextOutcomeV1 {
                input: Box::new(Input::Preview {
                    cbor_base64: encode_payload(&capture.input).unwrap(),
                }),
                decision: Decision::Previewed {
                    decision_cbor_base64: encode_payload(&decision).unwrap(),
                },
            }),
        }
    })
    .collect();
    (request, outputs)
}
fn spawn(
    fixture: &Fixture,
    request: PaperContextRequest,
) -> tokio::task::JoinHandle<Result<crate::ai::paper_reading_contracts::PaperContextPreview, Value>>
{
    let store = fixture.store.clone();
    tokio::spawn(async move {
        crate::ai::paper_context::preview::preview_context(&store, request).await
    })
}
#[tokio::test]
async fn changed_prefix_after_native_wait_rejects_without_later_queries_or_writes() {
    let mut f = Fixture::new();
    let (request, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    {
        let db = f.store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update paper_context_snapshots set context_schema_version=2",
            [],
        )
        .unwrap();
    }
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error, json!("conflict"));
    assert_eq!(f.counts(), (1, 0));
    assert!(!f.lifecycle().contains("call 1"));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_preview_waiter_retains_owner_and_releases_database_before_exchange() {
    let mut f = Fixture::new();
    let (request, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    {
        let db = f.store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update bibliographic_items set title='Core remains available' where id='paper'",
            [],
        )
        .unwrap();
    }
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            if f.service.doi_operations.try_write().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(f.lifecycle().contains("call 2"));
    assert_eq!(f.counts(), (1, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn shutdown_during_preview_keeps_actual_frame_identity_and_no_snapshot_writes() {
    let mut f = Fixture::new();
    let (request, outputs) = prepare(&f);
    f.configure(&outputs, Some(0), false);
    let caller = spawn(&f, request);
    assert!(f.entered(0).await);
    let identity = f
        .lifecycle()
        .lines()
        .find_map(|line| line.strip_prefix("request ").map(str::to_owned))
        .unwrap();
    let drained = tokio::time::timeout(WAIT, f.service.shutdown()).await;
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    drained.unwrap().unwrap();
    assert_eq!(error["requestId"], identity);
    assert_eq!(error["packageKey"], f.key);
    assert_eq!(f.counts(), (1, 0));
    assert!(f.store.connection().unwrap().is_autocommit());
}
#[tokio::test]
async fn unconfirmed_preview_rollback_fences_owner_before_host_exchange() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let mut f = Fixture::new();
    let (request, outputs) = prepare(&f);
    f.configure(&outputs, None, false);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = crate::ai::paper_context::preview::preview_context(&f.store, request)
        .await
        .unwrap_err();
    assert_eq!(error["nativeFailure"]["outcome"], "unknown", "{error}");
    assert!(
        error["nativeFailure"]["rollbackError"].is_object(),
        "{error}"
    );
    assert!(f.store.connection().is_err());
    assert!(!f.lifecycle().contains("call 0"));
    f.service.shutdown().await.unwrap();
}
