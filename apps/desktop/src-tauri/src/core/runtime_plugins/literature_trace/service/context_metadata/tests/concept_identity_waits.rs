//! Controlled waits use the original Core identity I/O and finite Native rules.
use super::*;
use crate::runtime_plugins::literature_knowledge_promotion::{
    KnowledgeIo, KnowledgePolicy, KnowledgeSql, PromotionError, identity,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::CatalogOutput,
    knowledge_promotion::{
        KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input, identity::*,
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
        let result = super::wait_policy_fixtures::knowledge("concept_identity_waits", &input);
        if let Some(previous) = self.0.insert(ordinal, input.clone()) {
            assert_eq!(previous, input);
        }
        Ok(result)
    }
    fn register_semantic(
        &mut self,
        _tx: &rusqlite::Transaction<'_>,
        _concept: &research_workbench_native_sdk::concepts::Concept,
    ) -> Result<(), PromotionError<Infallible>> {
        Ok(())
    }
}
fn fixture(split: bool) -> (Fixture, IdentityRequestV1, usize) {
    let mut f = Fixture::new();
    super::matrix_fixture::seed(&f);
    f.store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,created_at,updated_at) values('identity-survivor','Survivor','','now','now'),('identity-source','Source','','now','now'),('identity-third','Third','','now','now');
    insert into terms(id,text,normalized_text,language,created_at,updated_at) values('survivor-term','Survivor','survivor','en','now','now'),('source-term','Source','source','en','now','now');
    insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('identity-survivor','survivor-term','preferred','now'),('identity-source','source-term','preferred','now');").unwrap();
    let request = if split {
        let summary = |id: &str, label: &str| json!({"id":id,"label":label,"revision":1,"termCount":1,"definitionCount":0,"sourceCount":0});
        let snapshot = json!({"survivor":summary("identity-survivor","Survivor"),"sources":[summary("identity-source","Source")],"affectedRelationIds":[]});
        let db = f.store.connection().unwrap();
        db.execute("insert into concept_identity_events(id,operation,survivor_concept_id,source_concept_ids_json,snapshot_json,created_at) values('identity-merge','merge','identity-survivor','[\"identity-source\"]',?1,'now')",[snapshot.to_string()]).unwrap();
        db.execute("insert into concept_identity_mappings(source_concept_id,canonical_concept_id,merge_event_id,created_at) values('identity-source','identity-survivor','identity-merge','now')",[]).unwrap();
        IdentityRequestV1::Split {
            draft: ConceptSplitDraft {
                merge_event_id: "identity-merge".into(),
                source_concept_ids: vec!["identity-source".into()],
                rationale: "Split".into(),
            },
        }
    } else {
        IdentityRequestV1::Merge {
            draft: ConceptMergeDraft {
                survivor_id: "identity-survivor".into(),
                survivor_expected_revision: 1,
                sources: vec![ConceptMergeSourceDraft {
                    id: "identity-source".into(),
                    expected_revision: 1,
                }],
                rationale: "Merge".into(),
            },
        }
    };
    let sql = KnowledgeSql::for_test(super::wait_policy_fixtures::KNOWLEDGE_SQL);
    let mut policy = Recorded(BTreeMap::new());
    {
        let mut db = f.store.connection().unwrap();
        identity::precheck(&mut policy, &request).unwrap();
        let tx = db.transaction().unwrap();
        KnowledgeIo::new(&tx, &sql, &mut policy)
            .write_identity(&request)
            .unwrap();
        tx.rollback().unwrap();
    }
    let gate = if split { 3 } else { 6 };
    assert!(policy.0.contains_key(&gate));
    let outputs = policy
        .0
        .into_iter()
        .map(|(ordinal, input)| {
            let decision = WorkflowDecisionV1::KnowledgeValidation {
                decision: super::wait_policy_fixtures::knowledge("concept_identity_waits", &input),
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
    f.configure(&outputs, Some(gate), false);
    f.bind();
    (f, request, gate)
}
fn rows(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from concept_identity_events),(select count(*) from concept_identity_mappings),(select count(*) from research_object_revisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
fn spawn(
    f: &Fixture,
    request: IdentityRequestV1,
) -> tokio::task::JoinHandle<crate::runtime_plugins::native::NativeResult<IdentityOutputV1>> {
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move { service.concept_identity(store, key, request).await })
}
#[tokio::test]
async fn concept_identity_merge_wait_releases_guard_and_rejects_changed_summary() {
    let (f, request, gate) = fixture(false);
    let call = spawn(&f, request);
    if !f.entered(gate).await {
        f.release();
        panic!("merge gate not reached: {:?}", call.await);
    }
    assert_eq!(rows(&f), (0, 0, 0));
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    db.execute(
        "update concepts set label='Changed' where id='identity-source'",
        [],
    )
    .unwrap();
    drop(db);
    f.release();
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_source_changed");
    assert!(error.request_id.is_some());
    assert_eq!(rows(&f), (0, 0, 0));
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn concept_identity_split_wait_rolls_back_prefix_and_rejects_changed_mapping() {
    let (f, request, gate) = fixture(true);
    let call = spawn(&f, request);
    if !f.entered(gate).await {
        f.release();
        panic!("split gate not reached: {:?}", call.await);
    }
    assert_eq!(rows(&f), (1, 1, 0));
    let mut db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    let request = IdentityRequestV1::Split {
        draft: ConceptSplitDraft {
            merge_event_id: "identity-merge".into(),
            source_concept_ids: vec!["identity-source".into()],
            rationale: "Competing split".into(),
        },
    };
    let sql = KnowledgeSql::for_test(super::wait_policy_fixtures::KNOWLEDGE_SQL);
    let mut concurrent = Recorded(BTreeMap::new());
    identity::precheck(&mut concurrent, &request).unwrap();
    let tx = db.transaction().unwrap();
    KnowledgeIo::new(&tx, &sql, &mut concurrent)
        .write_identity(&request)
        .unwrap();
    tx.commit().unwrap();
    drop(db);
    let external = rows(&f);
    assert_eq!(external, (2, 0, 2));
    f.release();
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_source_changed");
    assert_eq!(rows(&f), external);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn concept_identity_aborted_caller_keeps_owner_and_shutdown_drains() {
    use std::{future::Future, task::Poll};
    for shutdown in [false, true] {
        let (f, request, gate) = fixture(false);
        let call = spawn(&f, request);
        if !f.entered(gate).await {
            f.release();
            panic!("identity gate not reached: {:?}", call.await);
        }
        assert_eq!(rows(&f), (0, 0, 0));
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
            assert_eq!(rows(&f), (0, 0, 0));
        } else {
            f.release();
            tokio::time::timeout(WAIT, async {
                while rows(&f).0 == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            f.service.shutdown().await.unwrap();
            assert_eq!(rows(&f), (1, 1, 1));
        }
    }
}
