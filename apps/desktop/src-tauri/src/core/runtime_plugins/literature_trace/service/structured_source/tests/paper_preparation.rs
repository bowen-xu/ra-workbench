//! Real Core preparation attempts over the original bounded host fixture.
use super::*;
use crate::ai::paper_reading_contracts::PrepareRequest;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    paper_context::{SnapshotProjectionInputV1, SnapshotProjectionV1},
    paper_context_prepare::PaperPreparationInputV1 as Policy,
    paper_context_wire::{
        PaperContextDecisionV1 as Decision, PaperContextInputV1 as Input, decode_payload,
        encode_payload,
    },
};
fn request() -> PrepareRequest {
    PrepareRequest {
        paper_id: "paper".into(),
        attachment_ids: vec!["selected".into()],
        image_assets: vec![],
    }
}
fn outputs(f: &Fixture) -> Vec<CatalogOutput> {
    let package = f.service.catalog.discover().unwrap().packages.remove(0);
    let resources = PaperContextResources::load(&f.service.catalog, &package).unwrap();
    let capture = resources
        .capture_build(
            &f.store.connection().unwrap(),
            "paper",
            &["selected".into()],
        )
        .unwrap();
    let prepared = crate::runtime_plugins::literature_context::test_fixture::build(&capture.input);
    let projection = SnapshotProjectionInputV1 {
        paper_id: "paper".into(),
        attachment_statuses: prepared.attachment_statuses.clone(),
        context_items: prepared.context_items,
        image_hashes: Default::default(),
    };
    [
        Input::Build {
            cbor_base64: encode_payload(&capture.input).unwrap(),
        },
        Input::Prepare {
            cbor_base64: encode_payload(&Policy::SnapshotReady {
                attachment_statuses: prepared.attachment_statuses,
            })
            .unwrap(),
        },
        Input::Project {
            cbor_base64: encode_payload(&projection).unwrap(),
        },
    ]
    .into_iter()
    .map(|input| {
        catalog_response(CatalogInput::PreparePaperContext {
            input: Box::new(input),
        })
        .unwrap()
    })
    .collect()
}
fn spawn(
    f: &Fixture,
) -> tokio::task::JoinHandle<Result<crate::ai::paper_reading_contracts::Preparation, Value>> {
    let store = f.store.clone();
    tokio::spawn(async move {
        crate::ai::paper_context::prepare::prepare_context(&store, request()).await
    })
}
#[tokio::test]
async fn snapshot_preparation_keeps_owner_and_database_available_after_waiter_cancels() {
    let mut f = Fixture::new();
    let expected = outputs(&f);
    let CatalogOutput::PaperContext { outcome } = &expected[2] else {
        panic!("projected output")
    };
    let Decision::Projected {
        projection_cbor_base64,
    } = &outcome.decision
    else {
        panic!("projection")
    };
    let projection: SnapshotProjectionV1 = decode_payload(projection_cbor_base64).unwrap();
    f.configure(&expected, Some(0), false);
    let caller = spawn(&f);
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
    assert_eq!(f.counts(), (1, 0));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select id from paper_context_snapshots", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        projection.snapshot_id
    );
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn raw_parse_changes_before_native_build_returns_reject_before_image_or_snapshot_work() {
    let mut f = Fixture::new();
    let expected = outputs(&f);
    f.configure(&expected, Some(0), false);
    let caller = spawn(&f);
    assert!(f.entered(0).await);
    f.store.connection().unwrap().execute("update docling_parse_runs set json_content=json_content || ' ' where attachment_id='selected'", []).unwrap();
    f.release();
    let error = tokio::time::timeout(WAIT, caller)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");
    assert_eq!(f.counts(), (0, 0));
    assert!(!f.lifecycle().contains("call 1"));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn primary_model_failure_precedes_legacy_session_reads_and_keeps_sql_diagnostics() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut f = Fixture::new();
    let expected = outputs(&f);
    f.configure(&expected[..1], None, false);
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&reads);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "llm_model_routes",
                ..
            } => Authorization::Deny,
            AuthAction::Read {
                table_name: "paper_ai_reading_sessions",
                ..
            } => {
                observed.fetch_add(1, Ordering::SeqCst);
                Authorization::Allow
            }
            _ => Authorization::Allow,
        }));
    let error = crate::ai::paper_context::prepare::prepare_reading_context(
        &f.store,
        "paper".into(),
        vec!["selected".into()],
    )
    .await
    .unwrap_err();
    assert_eq!(error["code"], "storage_error", "{error}");
    assert_eq!(
        error["nativeFailure"]["databaseError"]["sqliteExtendedCode"],
        23
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.counts(), (0, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn fallback_model_failure_stays_ignored_in_native_reading_preparation() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut f = Fixture::new();
    let expected = outputs(&f);
    f.configure(&expected[..1], None, false);
    {
        let db = f.store.connection().unwrap();
        db.execute("delete from llm_model_routes", []).unwrap();
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&reads);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "llm_model_routes",
                column_name: "route_kind",
                ..
            } => {
                if observed.fetch_add(1, Ordering::SeqCst) == 1 {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            _ => Authorization::Allow,
        }));
    let prepared = crate::ai::paper_context::prepare::prepare_reading_context(
        &f.store,
        "paper".into(),
        vec!["selected".into()],
    )
    .await
    .unwrap();
    assert!(prepared.model.is_none());
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(f.counts(), (0, 0));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn legacy_session_sql_failure_keeps_original_storage_code_and_resource_diagnostics() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let mut f = Fixture::new();
    let expected = outputs(&f);
    f.configure(&expected[..1], None, false);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "paper_ai_reading_sessions",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = crate::ai::paper_context::prepare::prepare_reading_context(
        &f.store,
        "paper".into(),
        vec!["selected".into()],
    )
    .await
    .unwrap_err();
    assert_eq!(error["code"], "storage_error", "{error}");
    assert_eq!(
        error["nativeFailure"]["dataError"]["resourceId"],
        "literature.paper-context.session-latest"
    );
    assert_eq!(
        error["nativeFailure"]["dataError"]["sqliteExtendedCode"],
        23
    );
    assert_eq!(f.counts(), (0, 0));
    assert!(!f.lifecycle().contains("call 1"));
    f.service.shutdown().await.unwrap();
}

fn catalog_response(input: CatalogInput) -> Result<CatalogOutput, String> {
    Ok(crate::runtime_plugins::literature_context::test_fixture::policy_catalog(input))
}
