//! Transport failures and shutdown use the same Core-owned accepted read lifetime.
use super::*;
use crate::runtime_plugins::literature_analysis::{
    commands, manager::PaperAnalysisWorkflowManager,
};
use research_workbench_native_sdk::{
    analysis_history::{
        HistoryInputV1, HistoryRequestV1, HistoryResultV1, HistoryWireInputV1, HistoryWireOutcomeV1,
    },
    catalog::CatalogOutput,
    paper_context_wire::encode_payload,
};
use std::fs;

fn request() -> HistoryRequestV1 {
    HistoryRequestV1::Latest {
        paper_id: "paper".into(),
    }
}
fn response() -> CatalogOutput {
    CatalogOutput::AnalysisHistory {
        outcome: Box::new(HistoryWireOutcomeV1 {
            input: HistoryWireInputV1 {
                cbor_base64: encode_payload(&HistoryInputV1 {
                    request: request(),
                    workflow: None,
                })
                .unwrap(),
            },
            result_cbor_base64: encode_payload(&HistoryResultV1::Ok(None)).unwrap(),
        }),
    }
}
fn configure(f: &mut Fixture, outputs: &[CatalogOutput], gate: Option<usize>) {
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.analysis-history","version":1}));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in ["workflow", "latest", "turns", "attempts"] {
        let file = format!("analysis-history-{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    f.configure(outputs, gate, false);
}
async fn read(
    f: &Fixture,
) -> Result<Option<crate::ai::paper_analysis_contracts::PaperAnalysisWorkflow>, Value> {
    commands::read_selected(
        f.store.clone(),
        PaperAnalysisWorkflowManager::default(),
        request(),
    )
    .await
}

#[tokio::test]
async fn history_absent_capability_and_changed_resource_fail_before_host_without_fallback() {
    let mut f = Fixture::new();
    f.bind();
    let failure = read(&f).await.unwrap_err();
    assert_eq!(failure["code"], "plugin_host_capability_unavailable");
    assert!(f.service.host.lock().await.is_none());
    configure(&mut f, &[response()], None);
    fs::write(
        f.package
            .path()
            .join("writing/data/analysis-history-latest.v1.sql"),
        "select 'other'",
    )
    .unwrap();
    let failure = read(&f).await.unwrap_err();
    assert_eq!(failure["code"], "plugin_host_package_unavailable");
    assert_eq!(failure["nativeFailure"]["outcome"], "notStarted");
    assert!(f.service.host.lock().await.is_none());
    f.bind();
    let failure = read(&f).await.unwrap_err();
    assert_eq!(failure["code"], "runtime_plugin_data_digest_mismatch");
    assert_eq!(
        failure["nativeFailure"]["dataError"]["resourceId"],
        "literature.analysis-history.latest"
    );
    assert!(f.service.host.lock().await.is_none());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn history_rejects_wrong_echo_kind_and_result_tree_with_actual_request_identity() {
    for mode in 0..3 {
        let mut f = Fixture::new();
        let mut wrong = response();
        if let CatalogOutput::AnalysisHistory { outcome } = &mut wrong {
            match mode {
                0 => {
                    outcome.input.cbor_base64 = encode_payload(&HistoryInputV1 {
                        request: HistoryRequestV1::Latest {
                            paper_id: "other".into(),
                        },
                        workflow: None,
                    })
                    .unwrap()
                }
                1 => outcome.result_cbor_base64 = encode_payload(&HistoryResultV1::Err(
                    research_workbench_native_sdk::analysis_history::HistoryRejectionV1::NotFound,
                ))
                .unwrap(),
                _ => wrong = f.response(),
            }
        }
        configure(&mut f, &[wrong], None);
        let failure = read(&f).await.unwrap_err();
        assert_eq!(failure["code"], "plugin_host_response_mismatch");
        assert_eq!(failure["nativeFailure"]["packageKey"], f.key);
        assert!(failure["nativeFailure"]["instanceId"].as_str().is_some());
        assert!(
            failure["nativeFailure"]["nativeRequestId"]
                .as_str()
                .is_some()
        );
        assert!(f.service.host.lock().await.is_none());
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn history_detached_waiter_releases_database_and_shutdown_drains_native_owner() {
    let mut f = Fixture::new();
    configure(&mut f, &[response()], Some(0));
    let store = f.store.clone();
    let caller = tokio::spawn(async move {
        commands::read_selected(store, PaperAnalysisWorkflowManager::default(), request()).await
    });
    let entered = f.entered(0).await;
    caller.abort();
    let canceled = caller.await.is_err_and(|error| error.is_cancelled());
    let store = f.store.clone();
    let independent = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            store
                .connection()
                .unwrap()
                .execute(
                    "update bibliographic_items set title='Core read-wait probe' where id='paper'",
                    [],
                )
                .unwrap()
        }),
    )
    .await;
    let shutdown = tokio::time::timeout(WAIT, f.service.shutdown()).await;
    f.release();
    assert!(entered);
    assert!(canceled);
    assert_eq!(independent.unwrap().unwrap(), 1);
    shutdown.unwrap().unwrap();
    assert!(f.service.host.lock().await.is_none());
    assert_eq!(read(&f).await.unwrap_err()["code"], "plugin_host_stopping");
}
