//! Original semantic queue and bounded negative acceptance paths through the real package.
use super::*;
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_item_original_semantic_queue_is_atomic_and_replayed_once() {
    let (mut f, _, credentials, task) = super::super::super::suggestion_generation::setup();
    f.store.connection().unwrap().execute("update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4',base_url='https://dashscope.aliyuncs.com/compatible-mode/v1',timeout_ms=30000 where provider_id='qwen-embedding'",[]).unwrap();
    let candidate = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
    )
    .await;
    let request = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("create temp trigger fail_queue before insert on embedding_outbox_jobs begin select raise(abort,'synthetic queue rejection'); end").unwrap();
    let error = decide(&f, request.clone()).await.unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_queue")
        .unwrap();
    let accepted = decide(&f, request.clone()).await.unwrap();
    let concept = target(&accepted, "concept_1");
    let query = "select count(*) from semantic_representations s join embedding_records r on r.representation_id=s.id join embedding_outbox_jobs j on j.embedding_record_id=r.id where s.subject_id=?1 and s.subject_kind='concept' and j.status='queued'";
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(query, [&concept], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    decide(&f, request.clone()).await.unwrap();
    f = reopen_actual_fixture(f, false).await;
    decide(&f, request).await.unwrap();
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(query, [&concept], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_definition_and_relation_faults_restore_all_rows() {
    for (kind, table) in [
        ("definition_suggestion_v1", "definition_claims"),
        (
            "definition_suggestion_v1",
            "definition_claim_evidence_links",
        ),
        ("relation_suggestion_v1", "relation_assertions"),
        (
            "relation_suggestion_v1",
            "relation_assertion_evidence_links",
        ),
    ] {
        let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
        let first = qualify(
            &f,
            &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
        )
        .await;
        let first = decide(
            &f,
            request(
                &first,
                "concept_1",
                "accept_unchanged",
                Some(AiSuggestionResolutionDraft::CreateConcept),
            ),
        )
        .await
        .unwrap();
        let first = target(&first, "concept_1");
        let mut value = payload("concept_suggestion_v1");
        value["suggestions"][0]["term"] = json!("Second endpoint");
        let second = qualify(&f, &generated(&f, &credentials, &task, value).await).await;
        let second = decide(
            &f,
            request(
                &second,
                "concept_1",
                "accept_unchanged",
                Some(AiSuggestionResolutionDraft::CreateConcept),
            ),
        )
        .await
        .unwrap();
        let second = target(&second, "concept_1");
        let candidate = qualify(&f, &generated(&f, &credentials, &task, payload(kind)).await).await;
        let (id, resolution) = if kind == "definition_suggestion_v1" {
            (
                "definition_1",
                AiSuggestionResolutionDraft::AttachDefinition { concept_id: first },
            )
        } else {
            (
                "relation_1",
                AiSuggestionResolutionDraft::AssertRelation {
                    subject_concept_id: first,
                    object_concept_id: second,
                    project_id: None,
                },
            )
        };
        let request = request(&candidate, id, "accept_unchanged", Some(resolution));
        let before = counts(&f);
        let revision_count = || {
            f.store
                .connection()
                .unwrap()
                .query_row("select count(*) from research_object_revisions", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        };
        let revisions = revision_count();
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_domain before insert on {table} begin select raise(abort,'synthetic failure'); end")).unwrap();
        assert_eq!(
            decide(&f, request.clone()).await.unwrap_err().code(),
            "storage_error"
        );
        assert_eq!(counts(&f), before);
        assert_eq!(revision_count(), revisions);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_domain")
            .unwrap();
        decide(&f, request).await.unwrap();
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_missing_targets_conflicts_and_incomplete_turn_write_nothing()
 {
    let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
    let candidate = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
    )
    .await;
    let before = counts(&f);
    let missing = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::LinkConcept {
            concept_id: "absent".into(),
        }),
    );
    // Original alias insertion reports the missing FK before the later concept read.
    let expected = "storage_error";
    assert_eq!(
        decide(&f, missing).await.unwrap_err().code(),
        expected
    );
    assert_eq!(counts(&f), before);
    let mut stale = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    stale.candidate_revision += 1;
    assert_eq!(decide(&f, stale).await.unwrap_err().code(), "invalid_input");
    assert_eq!(counts(&f), before);
    let accepted = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    decide(&f, accepted.clone()).await.unwrap();
    let after = counts(&f);
    let mut collision = accepted;
    collision.rationale.push('x');
    assert_eq!(
        decide(&f, collision).await.unwrap_err().code(),
        "invalid_input"
    );
    assert_eq!(counts(&f), after);
    close(f).await;
    // A mapped current run has no historical exemption from the shared-turn guard.
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let mut value = payload("concept_suggestion_v1");
    value["suggestions"][0]["evidenceItemIds"] = json!(["ev-quote"]);
    let candidate = super::super::super::generic_reading::seed(&f, value);
    f.bind_actual_package(catalog, key);
    f.store.connection().unwrap().execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind,candidate_id) values('legacy-run',?1,?2,'knowledge_suggestion','legacy-candidate')",params![f.record.conversation_id.to_string(),f.record.request_id.to_string()]).unwrap();
    let before = counts(&f);
    assert_eq!(
        decide(&f, request(&candidate, "concept_1", "defer", None))
            .await
            .unwrap_err()
            .code(),
        "temporary_unavailable"
    );
    assert_eq!(counts(&f), before);
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_keep_canonical_identity_and_relation_error_order() {
    use crate::runtime_plugins::literature_concepts::{ConceptRequest, ConceptResult};
    use research_workbench_native_sdk::concepts::ConceptDraft;
    let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
    let mut concepts = Vec::new();
    for name in ["Surviving concept", "Merged concept"] {
        let result = f
            .service
            .concepts(
                f.store.shared_agent_connection(),
                f.key.clone(),
                ConceptRequest::Save {
                    draft: Box::new(ConceptDraft {
                        id: None,
                        preferred_term: name.into(),
                        language: Some("en".into()),
                        label: name.into(),
                        explanation: "Synthetic identity".into(),
                        disambiguation: String::new(),
                        domain: String::new(),
                        tradition: String::new(),
                        period: String::new(),
                        expected_revision: None,
                        change_reason: None,
                    }),
                },
            )
            .await
            .unwrap();
        let ConceptResult::Concept(concept) = result else {
            panic!("concept save")
        };
        concepts.push(concept);
    }
    // Existing persisted mapping is the input here; merge execution retains its own tests.
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into concept_identity_events(id,operation,survivor_concept_id,source_concept_ids_json,snapshot_json,rationale,created_at) values('synthetic-history','merge',?1,?2,'{}','Synthetic historical mapping','old')", params![concepts[0].id, serde_json::to_string(&vec![&concepts[1].id]).unwrap()]).unwrap();
        db.execute("insert into concept_identity_mappings(source_concept_id,canonical_concept_id,merge_event_id,created_at) values(?1,?2,'synthetic-history','old')", params![concepts[1].id,concepts[0].id]).unwrap();
    }
    let candidate = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
    )
    .await;
    let accepted = decide(
        &f,
        request(
            &candidate,
            "concept_1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::LinkConcept {
                concept_id: concepts[1].id.clone(),
            }),
        ),
    )
    .await
    .unwrap();
    assert_eq!(target(&accepted, "concept_1"), concepts[0].id);
    let relation = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("relation_suggestion_v1")).await,
    )
    .await;
    let before = counts(&f);
    let same = request(
        &relation,
        "relation_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AssertRelation {
            subject_concept_id: concepts[1].id.clone(),
            object_concept_id: concepts[0].id.clone(),
            project_id: Some("absent".into()),
        }),
    );
    let error = decide(&f, same).await.unwrap_err();
    assert!(error.to_string().contains("关系的起点和终点不能是同一概念"));
    assert_eq!(counts(&f), before);
    let missing_project = request(
        &relation,
        "relation_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AssertRelation {
            subject_concept_id: concepts[0].id.clone(),
            object_concept_id: "missing endpoint".into(),
            project_id: Some("absent".into()),
        }),
    );
    let error = decide(&f, missing_project).await.unwrap_err();
    assert_eq!(error.code(), "invalid_input");
    assert_eq!(counts(&f), before);
    close(f).await;
}
