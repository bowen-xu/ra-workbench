//! Controlled native waits exercise the actual Core owner and finite original rule producer.
use super::*;
use crate::runtime_plugins::literature_knowledge_promotion::{
    KnowledgePolicy, KnowledgeSql, PromotionError, foundations,
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
        let decision = super::wait_policy_fixtures::knowledge("shared_foundations_waits", &input);
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
    let sql = KnowledgeSql::for_test(super::wait_policy_fixtures::KNOWLEDGE_SQL);
    let mut policy = Recorded(BTreeMap::new());
    {
        let mut guard = f.store.connection().unwrap();
        let tx = guard.transaction().unwrap();
        foundations::prepare_start(&tx, &sql, &mut policy, &draft()).unwrap();
        foundations::prepare_results(&tx, &sql, &mut policy, &draft()).unwrap();
        tx.rollback().unwrap();
    }
    let outputs = policy
        .0
        .into_iter()
        .map(|(ordinal, input)| {
            let decision = WorkflowDecisionV1::KnowledgeValidation {
                decision: super::wait_policy_fixtures::knowledge(
                    "shared_foundations_waits",
                    &input,
                ),
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
    super::wait_policy_fixtures::install_knowledge_sql(&directory);
    f.configure(&outputs, Some(1), false);
    f.bind();
    f
}
fn rows(f: &Fixture) -> (i64, i64, String) {
    f.store.connection().unwrap().query_row("select (select count(*) from concept_algorithm_runs),(select count(*) from paper_foundation_similarity_results),coalesce((select status from concept_algorithm_runs order by id limit 1),'none')", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
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
    tokio::spawn(async move { service.shared_foundations(store, key, draft()).await })
}

#[tokio::test]
async fn shared_foundations_wait_releases_guard_and_preserves_failed_run_on_source_change() {
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
async fn shared_foundations_caller_abort_keeps_owned_completion_and_shutdown_drains() {
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
            assert_eq!(rows(&f), (1, 3, "succeeded".into()));
            f.service.shutdown().await.unwrap();
        }
    }
}

fn draft() -> research_workbench_native_sdk::knowledge_promotion::algorithms::SharedFoundationsDraft
{
    research_workbench_native_sdk::knowledge_promotion::algorithms::SharedFoundationsDraft {
        paper_ids: vec!["paper-1".into(), "paper-2".into(), "paper-3".into()],
    }
}
