//! Original relation preflight owns no transaction or database guard during Native waits.
use super::*;
use crate::runtime_plugins::literature_knowledge_promotion::{
    KnowledgePolicy, KnowledgeSql, PromotionError, relation,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::CatalogOutput,
    knowledge_promotion::{
        KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input,
        relation::RelationAssertionDraft,
    },
    paper_context_wire::encode_payload,
};
use std::{convert::Infallible, fs};
struct Recorded(Vec<Input>);
impl KnowledgePolicy for Recorded {
    type Pause = Infallible;
    fn decide(
        &mut self,
        _ordinal: usize,
        input: Input,
    ) -> Result<Decision, PromotionError<Infallible>> {
        let decision = super::wait_policy_fixtures::knowledge("manual_relation_waits", &input);
        self.0.push(input);
        Ok(decision)
    }
    fn register_semantic(
        &mut self,
        _tx: &rusqlite::Transaction<'_>,
        _concept: &research_workbench_native_sdk::concepts::Concept,
    ) -> Result<(), PromotionError<Infallible>> {
        unreachable!("relations do not register semantic objects")
    }
}
fn fixture() -> (Fixture, RelationAssertionDraft) {
    let mut f = Fixture::new();
    super::matrix_fixture::seed(&f);
    f.store.connection().unwrap().execute_batch("insert into note_pages(id,title,position,status,is_favorite,revision,schema_version,created_at,updated_at) values('relation-note','Relation note',0,'active',0,0,1,'now','now');").unwrap();
    let draft:RelationAssertionDraft=serde_json::from_value(json!({"subject":{"kind":"paper","id":"paper"},"predicateKind":"related_to","object":{"kind":"note_page","id":"relation-note"},"scopeNote":" Scope "})).unwrap();
    let mut policy = Recorded(Vec::new());
    let sql = KnowledgeSql::for_test(super::wait_policy_fixtures::KNOWLEDGE_SQL);
    relation::prepare_relation(&f.store.connection().unwrap(), &sql, &mut policy, &draft).unwrap();
    let outputs = policy
        .0
        .into_iter()
        .enumerate()
        .map(|(ordinal, input)| {
            let decision = WorkflowDecisionV1::KnowledgeValidation {
                decision: super::wait_policy_fixtures::knowledge("manual_relation_waits", &input),
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
    f.configure(&outputs, Some(3), false);
    f.bind();
    (f, draft)
}
fn rows(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from relation_assertions", [], |r| r.get(0))
        .unwrap()
}
fn spawn(
    f: &Fixture,
    draft: RelationAssertionDraft,
) -> tokio::task::JoinHandle<
    crate::runtime_plugins::native::NativeResult<
        research_workbench_native_sdk::knowledge_promotion::relation::RelationAssertion,
    >,
> {
    let owner = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move { owner.save_relation(store, key, draft).await })
}
#[tokio::test]
async fn manual_relation_wait_releases_guard_before_begin_and_rejects_changed_node() {
    let (f, draft) = fixture();
    let call = spawn(&f, draft);
    if !f.entered(3).await {
        f.release();
        panic!("relation gate not reached: {:?}", call.await);
    }
    assert_eq!(rows(&f), 0);
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    db.execute(
        "update note_pages set title='Changed during wait' where id='relation-note'",
        [],
    )
    .unwrap();
    drop(db);
    f.release();
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_source_changed");
    assert!(error.request_id.is_some());
    assert_eq!(rows(&f), 0);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn manual_relation_aborted_caller_keeps_owner_and_shutdown_drains() {
    use std::{future::Future, task::Poll};
    for shutdown in [false, true] {
        let (f, draft) = fixture();
        let call = spawn(&f, draft);
        if !f.entered(3).await {
            f.release();
            panic!("relation gate not reached: {:?}", call.await);
        }
        assert_eq!(rows(&f), 0);
        assert!(f.store.connection().unwrap().is_autocommit());
        call.abort();
        let _ = call.await;
        if shutdown {
            let mut stopping = Box::pin(f.service.shutdown());
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(stopping.as_mut().poll(cx).is_pending()))
                    .await
            );
            f.release();
            stopping.await.unwrap();
            assert_eq!(rows(&f), 0);
        } else {
            f.release();
            tokio::time::timeout(WAIT, async {
                while rows(&f) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            f.service.shutdown().await.unwrap();
            assert_eq!(rows(&f), 1);
        }
    }
}
