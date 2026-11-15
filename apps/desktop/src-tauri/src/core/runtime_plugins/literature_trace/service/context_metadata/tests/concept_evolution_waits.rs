//! Controlled native waits exercise the actual Core owner and finite original rule producer.
use super::*;
use crate::runtime_plugins::literature_knowledge_promotion::{
    KnowledgePolicy, KnowledgeSql, PromotionError, evolution,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::CatalogOutput,
    knowledge_promotion::{
        KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input,
    },
    paper_context_wire::encode_payload,
};
use std::{collections::BTreeMap, convert::Infallible, fs};

struct Recorded(BTreeMap<usize, Input>);
impl KnowledgePolicy for Recorded {
    type Pause = Infallible;
    fn decide(
        &mut self,
        ordinal: usize,
        input: Input,
    ) -> Result<Decision, PromotionError<Infallible>> {
        let decision = research_literature_library_native::knowledge_promotion::decide(&input);
        if let Some(previous) = self.0.insert(ordinal, input.clone()) {
            assert_eq!(previous, input);
        }
        Ok(decision)
    }
    fn register_semantic(
        &mut self,
        _tx: &rusqlite::Transaction<'_>,
        _concept: &research_workbench_native_sdk::concepts::Concept,
    ) -> Result<(), PromotionError<Infallible>> {
        panic!("Original evolution never registers semantic representations");
    }
}
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    super::evolution_fixture::seed(&f);
    let sql = KnowledgeSql::for_test(
        &research_literature_library_native::knowledge_promotion::storage::static_sql(),
    );
    let mut policy = Recorded(BTreeMap::new());
    {
        let mut guard = f.store.connection().unwrap();
        let tx = guard.transaction().unwrap();
        evolution::start(&tx, &sql, &mut policy, "concept-shared").unwrap();
        evolution::prepare_events(&tx, &sql, &mut policy, "concept-shared").unwrap();
        tx.rollback().unwrap();
    }
    let outputs = policy
        .0
        .into_iter()
        .map(|(ordinal, input)| {
            let decision = WorkflowDecisionV1::KnowledgeValidation {
                decision: research_literature_library_native::knowledge_promotion::decide(&input),
            };
            let input = WorkflowInputV1::KnowledgeValidation {
                ordinal,
                input: Box::new(input),
            };
            CatalogOutput::AnalysisWorkflow {
                outcome: Box::new(WorkflowWireOutcomeV1 {
                    input: WorkflowWireInputV1 {
                        cbor_base64: encode_payload(&input).unwrap(),
                    },
                    decision_cbor_base64: encode_payload(&decision).unwrap(),
                }),
            }
        })
        .collect::<Vec<_>>();
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    for id in ["literature.analysis-workflow", "literature.concepts"] {
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for entry in fs::read_dir(resources).unwrap() {
        let entry = entry.unwrap();
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "sql")
        {
            fs::copy(entry.path(), directory.join("data").join(entry.file_name())).unwrap();
        }
    }
    f.configure(&outputs, Some(1), false);
    f.bind();
    f
}
fn rows(f: &Fixture) -> (i64, i64, String) {
    f.store.connection().unwrap().query_row("select (select count(*) from concept_algorithm_runs),(select count(*) from concept_evolution_results),coalesce((select status from concept_algorithm_runs order by id limit 1),'none')", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}
fn spawn(
    f: &Fixture,
) -> tokio::task::JoinHandle<
    crate::runtime_plugins::native::NativeResult<
        research_workbench_native_sdk::knowledge_promotion::algorithms::ConceptAlgorithmRun,
    >,
> {
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move {
        service
            .concept_evolution(store, key, "concept-shared".into())
            .await
    })
}

#[tokio::test]
async fn concept_evolution_wait_releases_guard_and_preserves_failed_run_on_source_change() {
    let f = fixture();
    let call = spawn(&f);
    if !f.entered(1).await {
        f.release();
        panic!("Evolution gate not reached: {:?}", call.await);
    }
    assert_eq!(rows(&f), (1, 0, "running".into()));
    {
        let guard = f.store.connection().unwrap();
        assert!(guard.is_autocommit());
        guard
            .execute(
                "update concepts set label='Changed Ω' where id='concept-shared'",
                [],
            )
            .unwrap();
    }
    f.release();
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_source_changed");
    assert!(error.request_id.is_some());
    assert!(error.concept_algorithm.unwrap().object_id.is_some());
    assert_eq!(rows(&f), (1, 0, "failed".into()));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn concept_evolution_caller_abort_keeps_owned_completion_and_shutdown_drains() {
    use std::{future::Future, task::Poll};
    for shutdown in [false, true] {
        let f = fixture();
        let call = spawn(&f);
        if !f.entered(1).await {
            f.release();
            panic!("Evolution gate not reached: {:?}", call.await);
        }
        call.abort();
        if shutdown {
            let stopping = f.service.shutdown();
            tokio::pin!(stopping);
            assert!(matches!(
                std::future::poll_fn(|context| Poll::Ready(stopping.as_mut().poll(context))).await,
                Poll::Pending
            ));
            f.release();
            stopping.await.unwrap();
            assert_eq!(rows(&f), (1, 0, "failed".into()));
        } else {
            f.release();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                if rows(&f).2 == "succeeded" {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "Owned evolution did not settle"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(rows(&f), (1, 2, "succeeded".into()));
            f.service.shutdown().await.unwrap();
        }
    }
}

#[test]
fn concept_evolution_reuses_equal_classification_observations_without_limiting_result_rows() {
    let f = Fixture::new();
    super::evolution_fixture::seed(&f);
    let sql = KnowledgeSql::for_test(
        &research_literature_library_native::knowledge_promotion::storage::static_sql(),
    );
    let mut db = f.store.connection().unwrap();
    db.execute_batch("pragma foreign_keys=off;").unwrap();
    for index in 0..130 {
        super::evolution_fixture::insert_assertion(
            &db,
            &format!("transport-{index:03}"),
            "paper-1",
            "concept-shared",
            "focal",
            "unclear",
            "Original repeated classification",
        );
    }
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    let tx = db.transaction().unwrap();
    let mut policy = Recorded(BTreeMap::new());
    evolution::start(&tx, &sql, &mut policy, "concept-shared").unwrap();
    let events = evolution::prepare_events(&tx, &sql, &mut policy, "concept-shared").unwrap();
    assert_eq!(events.len(), 132);
    assert_eq!(policy.0.len(), 2);
    let Input::EvolutionEvents {
        novelty_statuses, ..
    } = policy.0.get(&1).unwrap()
    else {
        panic!("classification observation")
    };
    assert_eq!(novelty_statuses.len(), 3);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_kind == "unclear")
            .count(),
        130
    );
    tx.rollback().unwrap();
}
