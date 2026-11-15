//! Original identity lifecycle through real Core and an independently built literature package.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::identity::*;
async fn fixture() -> Fixture {
    let f = evidence_matrix::fixture().await;
    f.store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,created_at,updated_at) values('identity-survivor','Survivor Ω','','now','now'),('identity-source','Source Ω','','now','now'),('identity-third','Third Ω','','now','now');
    insert into terms(id,text,normalized_text,language,created_at,updated_at) values('survivor-term','Survivor Ω','survivor ω','en','now','now'),('source-term','Source Ω','source ω','en','now','now'),('third-term','Third Ω','third ω','en','now','now');
    insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('identity-survivor','survivor-term','preferred','now'),('identity-source','source-term','preferred','now'),('identity-third','third-term','preferred','now');
    insert into definition_claims(id,concept_id,statement,scope_note,provenance_kind,epistemic_role,revision,created_at,updated_at) values('identity-definition','identity-source','Source definition','','human_original','definition',1,'now','now');
    insert into relation_assertions(id,subject_kind,subject_id,subject_label_snapshot,predicate_kind,object_kind,object_id,object_label_snapshot,scope_note,provenance_kind,epistemic_role,revision,created_at,updated_at) values('identity-relation','concept','identity-source','Source Ω','related_to','concept','identity-survivor','Survivor Ω','','human_original','assertion',1,'now','now');
    insert into semantic_representations(id,subject_kind,subject_id,subject_revision,representation_kind,namespace,language,render_version,rendered_text,input_sha256,created_at) values('identity-source-representation','concept','identity-source',1,'concept_identity','accepted_concepts','en','concept-identity-v1','Source Ω','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now');
    insert into embedding_spaces(id,provider_id,region,model_id,model_revision,dimension,scalar_type,distance_metric,normalization,document_input_role,query_input_role,instruction_set_version,created_at) values('identity-space','synthetic','','model','revision',2,'float32','cosine','none','document','query','v1','now');
    insert into embedding_index_generations(id,namespace,embedding_space_id,backend_kind,generation,status,relative_database_path,lance_table,index_kind,config_json,created_at) values('identity-gen-1','accepted_concepts','identity-space','lancedb',1,'building','semantic/identity-1','accepted_concepts','exact','{}','now'),('identity-gen-2','accepted_concepts','identity-space','lancedb',2,'building','semantic/identity-2','accepted_concepts','exact','{}','now');
    insert into embedding_records(id,representation_id,embedding_space_id,index_generation_id,lance_table,lance_row_id,vector_encoding,status,created_at,updated_at) values('identity-record-1','identity-source-representation','identity-space','identity-gen-1','accepted_concepts','identity-row-1','float32','queued','now','now'),('identity-record-2','identity-source-representation','identity-space','identity-gen-2','accepted_concepts','identity-row-2','float32','running','now','now');
    insert into embedding_outbox_jobs(id,embedding_record_id,attempt,status,idempotency_key,lease_owner,lease_expires_at,created_at) values('identity-job-1','identity-record-1',1,'queued','identity-job-1',null,null,'now'),('identity-job-2','identity-record-2',1,'running','identity-job-2','synthetic-owner','later','now');").unwrap();
    f
}
fn draft() -> ConceptMergeDraft {
    ConceptMergeDraft {
        survivor_id: "identity-survivor".into(),
        survivor_expected_revision: 1,
        sources: vec![ConceptMergeSourceDraft {
            id: "identity-source".into(),
            expected_revision: 1,
        }],
        rationale: " Reason Ω ".into(),
    }
}
fn split(id: &str) -> IdentityRequestV1 {
    IdentityRequestV1::Split {
        draft: ConceptSplitDraft {
            merge_event_id: id.into(),
            source_concept_ids: vec!["identity-source".into()],
            rationale: " Split Ω ".into(),
        },
    }
}
async fn call(
    f: &Fixture,
    request: IdentityRequestV1,
) -> crate::runtime_plugins::native::NativeResult<IdentityOutputV1> {
    f.service
        .concept_identity(f.store.clone(), f.key.clone(), request)
        .await
}
async fn merge(f: &Fixture) -> ConceptIdentityEvent {
    let IdentityOutputV1::Event { event } = call(f, IdentityRequestV1::Merge { draft: draft() })
        .await
        .unwrap()
    else {
        panic!("event")
    };
    event
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from concept_identity_events),(select count(*) from concept_identity_mappings),(select count(*) from concept_identity_relation_impacts),(select count(*) from research_object_revisions),(select count(*) from semantic_representations)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_identity_preview_merge_history_split_semantics_and_reopen() {
    let f = fixture().await;
    let IdentityOutputV1::Impact { impact } =
        call(&f, IdentityRequestV1::Preview { draft: draft() })
            .await
            .unwrap()
    else {
        panic!("impact")
    };
    assert_eq!(impact.sources[0].definition_count, 1);
    assert_eq!(impact.affected_relation_ids, vec!["identity-relation"]);
    assert_eq!(counts(&f), (0, 0, 0, 0, 1));
    let merged = merge(&f).await;
    assert_eq!(merged.operation, "merge");
    assert_eq!(merged.rationale, "Reason Ω");
    assert_eq!(merged.active_source_ids, vec!["identity-source"]);
    assert_eq!(counts(&f), (1, 1, 1, 1, 2));
    let definitions = f
        .service
        .list_definitions(f.store.clone(), f.key.clone(), "identity-survivor".into())
        .await
        .unwrap();
    assert_eq!(definitions[0].concept_id, "identity-source");
    let states:Vec<(String,String,Option<String>,Option<String>)>=f.store.connection().unwrap().prepare("select j.status,r.status,j.error_code,j.lease_owner from embedding_outbox_jobs j join embedding_records r on r.id=j.embedding_record_id order by j.id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(
        states,
        vec![
            (
                "canceled".into(),
                "tombstoned".into(),
                Some("concept_merged".into()),
                None
            );
            2
        ]
    );
    let IdentityOutputV1::Event { event: split_event } = call(&f, split(&merged.id)).await.unwrap()
    else {
        panic!("split event")
    };
    assert_eq!(split_event.operation, "split");
    assert!(split_event.active_source_ids.is_empty());
    assert_eq!(counts(&f), (2, 0, 2, 3, 4));
    let endpoints:(String,String)=f.store.connection().unwrap().query_row("select subject_id,subject_label_snapshot from relation_assertions where id='identity-relation'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(endpoints, ("identity-source".into(), "Source Ω".into()));
    assert!(
        f.service
            .list_definitions(f.store.clone(), f.key.clone(), "identity-survivor".into())
            .await
            .unwrap()
            .is_empty()
    );
    let history = call(&f, IdentityRequestV1::List {}).await.unwrap();
    let IdentityOutputV1::Events { events } = &history else {
        panic!("history")
    };
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .find(|e| e.id == merged.id)
            .unwrap()
            .active_source_ids
            .is_empty()
    );
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(call(&f, IdentityRequestV1::List {}).await.unwrap(), history);
    assert_eq!(counts(&f), (2, 0, 2, 3, 4));
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_identity_error_order_and_mapping_cas_keep_original_writes() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture().await;
    let begins = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&begins);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(move |c: AuthContext<'_>| {
            if matches!(c.action, AuthAction::Transaction { .. }) {
                seen.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }));
    let mut invalid = draft();
    invalid.sources.clear();
    invalid.rationale = "界".repeat(2_001);
    let error = call(
        &f,
        IdentityRequestV1::Merge {
            draft: invalid.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.message, "概念合并说明过长");
    assert!(error.request_id.is_some());
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&f, IdentityRequestV1::Preview { draft: invalid })
            .await
            .unwrap_err()
            .message,
        "概念合并参数无效"
    );
    let bad = ConceptSplitDraft {
        merge_event_id: "missing".into(),
        source_concept_ids: vec!["source".into(), "source".into()],
        rationale: "界".repeat(2_001),
    };
    assert_eq!(
        call(&f, IdentityRequestV1::Split { draft: bad.clone() })
            .await
            .unwrap_err()
            .message,
        "概念拆回参数无效"
    );
    let mut bad = bad;
    bad.rationale.clear();
    assert_eq!(
        call(&f, IdentityRequestV1::Split { draft: bad })
            .await
            .unwrap_err()
            .message,
        "概念拆回目标重复"
    );
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let mut preview = draft();
    preview.rationale = "界".repeat(2_001);
    assert!(matches!(
        call(&f, IdentityRequestV1::Preview { draft: preview })
            .await
            .unwrap(),
        IdentityOutputV1::Impact { .. }
    ));
    let mut stale = draft();
    stale.survivor_expected_revision = 2;
    let error = call(&f, IdentityRequestV1::Merge { draft: stale })
        .await
        .unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(1));
    assert!(error.request_id.is_some());
    assert_eq!(counts(&f), (0, 0, 0, 0, 1));
    let merged = merge(&f).await;
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("create temp trigger identity_delete_ignored before delete on concept_identity_mappings begin select raise(ignore); end;").unwrap();
    let error = call(&f, split(&merged.id)).await.unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "概念映射已变化，请重新预览");
    assert!(error.request_id.is_some());
    assert_eq!(counts(&f), before);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_identity_late_merge_and_split_failures_roll_back_the_complete_chain() {
    let f = fixture().await;
    let before = counts(&f);
    for table in [
        "concept_identity_mappings",
        "concept_identity_relation_impacts",
        "research_object_revisions",
        "semantic_representations",
    ] {
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger identity_fail before insert on {table} begin select raise(abort,'synthetic identity failure'); end;")).unwrap();
        let error = call(&f, IdentityRequestV1::Merge { draft: draft() })
            .await
            .unwrap_err();
        assert_eq!(error.code, "storage_error");
        assert_eq!(
            error.concept_identity.as_ref().unwrap().outcome,
            crate::hbs_storage::boundary::HbsOutcome::RolledBack
        );
        assert_eq!(counts(&f), before);
        let state: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select status from embedding_outbox_jobs where id='identity-job-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "running");
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger identity_fail")
            .unwrap();
    }
    let merged = merge(&f).await;
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("create temp trigger identity_fail before insert on research_object_revisions begin select raise(abort,'synthetic split failure'); end;").unwrap();
    let error = call(&f, split(&merged.id)).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger identity_fail")
        .unwrap();
    call(&f, split(&merged.id)).await.unwrap();
    assert_eq!(counts(&f).1, 0);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_identity_unknown_commit_fences_and_reopens() {
    let f = fixture().await;
    f.store.connection().unwrap().commit_hook(Some(|| true));
    let error = call(&f, IdentityRequestV1::Merge { draft: draft() })
        .await
        .unwrap_err();
    assert_eq!(error.code, "data_operation_outcome_unknown");
    assert!(f.store.connection().is_err());
    let f = reopen_actual_fixture(f, true).await;
    assert_eq!(counts(&f), (0, 0, 0, 0, 1));
    let merged = merge(&f).await;
    call(&f, split(&merged.id)).await.unwrap();
    assert_eq!(counts(&f), (2, 0, 2, 3, 4));
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_identity_committed_read_failure_retains_event_mapping_and_semantic_writes()
{
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = fixture().await;
    let committed = Arc::new(AtomicBool::new(false));
    {
        let db = f.store.connection().unwrap();
        let flag = Arc::clone(&committed);
        db.commit_hook(Some(move || {
            flag.store(true, Ordering::SeqCst);
            false
        }));
        let flag = Arc::clone(&committed);
        db.authorizer(Some(move |c: AuthContext<'_>| {
            if flag.load(Ordering::SeqCst)
                && matches!(
                    c.action,
                    AuthAction::Read {
                        table_name: "concept_identity_events",
                        ..
                    }
                )
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    }
    let error = call(&f, IdentityRequestV1::Merge { draft: draft() })
        .await
        .unwrap_err();
    let detail = error.concept_identity.unwrap();
    assert_eq!(
        detail.outcome,
        crate::hbs_storage::boundary::HbsOutcome::Committed
    );
    assert!(detail.object_id.is_some());
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(counts(&f), (1, 1, 1, 1, 2));
    let f = reopen_actual_fixture(f, false).await;
    let IdentityOutputV1::Events { events } = call(&f, IdentityRequestV1::List {}).await.unwrap()
    else {
        panic!("events")
    };
    assert_eq!(events.len(), 1);
    call(&f, split(&events[0].id)).await.unwrap();
    assert_eq!(counts(&f), (2, 0, 2, 3, 4));
    evidence_matrix::close(f).await;
}
