//! Synthetic HTTP only. SQL, Native decisions and the C ABI vector store are real.
use super::*;
use crate::{
    ai::{EmbeddingAccess, EmbeddingHttpWire, JsonCredentialStore, ProviderError},
    runtime_plugins::literature_retrieval::{commands, manager::ConceptRetrievalManager},
    semantic_engine::{SemanticNamespace, SemanticVectorRow, SemanticVectorStore, VectorAccess},
};
use research_workbench_native_sdk::concept_retrieval::{
    ConceptRetrievalPreview, ConceptRetrievalPreviewDraft,
};
use std::future::Future;
use std::pin::Pin;

#[derive(Default)]
struct SyntheticEmbedding {
    gate: Option<Arc<EmbeddingGate>>,
    rate_limited: bool,
}
#[derive(Default)]
struct EmbeddingGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl EmbeddingHttpWire for SyntheticEmbedding {
    fn respond(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = Result<reqwest::Response, ProviderError>> + Send + '_>> {
        let payload: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(request.url().scheme(), "https");
        assert_eq!(request.url().path(), "/compatible-mode/v1/embeddings");
        assert_eq!(payload["model"], "text-embedding-v4");
        assert_eq!(payload["dimensions"], 3);
        Box::pin(async move {
            if let Some(gate) = &self.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if self.rate_limited {
                return Ok(hyper::Response::builder()
                    .status(429)
                    .header("retry-after", "5")
                    .body(reqwest::Body::from("untrusted synthetic body"))
                    .unwrap()
                    .into());
            }
            Ok(hyper::Response::builder()
                .status(200)
                .body(reqwest::Body::from(
                    json!({
                        "model":"text-embedding-v4", "data":[{"index":0,"embedding":[1.0,0.0,0.0]}],
                        "usage":{"prompt_tokens":1,"total_tokens":1}
                    })
                    .to_string(),
                ))
                .unwrap()
                .into())
        })
    }
}

pub(super) async fn generate(f: &Fixture, paper: &str, query: &str) -> ConceptRetrievalPreview {
    assert!(f.store.retrieval_preview_source().is_none());
    seed(f).await;
    let embedding = embedding(f, SyntheticEmbedding::default());
    let preview = commands::preview_selected(
        f.store.clone(),
        embedding,
        ConceptRetrievalManager::default(),
        ConceptRetrievalPreviewDraft {
            client_request_id: "actual-generated-retrieval".into(),
            paper_id: Some(paper.into()),
            query: query.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(preview.seeds.len(), 1);
    assert_eq!(preview.seeds[0].concept_id, "retrieval-concept");
    assert_eq!(preview.seeds[0].matched_by, ["lexical", "vector"]);
    assert_eq!(preview.semantic_status, "used");
    assert_eq!(preview.sources.len(), 1);
    assert!(preview.sources[0].selected);
    assert_eq!(preview.sources[0].anchor_id, "retrieval-anchor");
    assert!(context(f, &preview.trace_id).contains("Evidence definition Ω"));
    assert_eq!(preview.external_paper_count, 0);
    let db = f.store.connection().unwrap();
    let leases: i64 = db
        .query_row(
            "select count(*) from embedding_generation_leases where owner_id=?1",
            [&preview.trace_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leases, 0);
    drop(db);
    let loaded = commands::read_selected(f.store.clone(), format!(" {} ", preview.trace_id))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&preview).unwrap()
    );
    preview
}

fn embedding(f: &Fixture, wire: SyntheticEmbedding) -> EmbeddingAccess {
    let credentials =
        JsonCredentialStore::from_path(f._data.path().join("synthetic-embedding.json"));
    credentials
        .set("qwen-embedding", "synthetic-retrieval-key")
        .unwrap();
    EmbeddingAccess::for_test(credentials, Arc::new(wire))
}
fn draft() -> ConceptRetrievalPreviewDraft {
    ConceptRetrievalPreviewDraft {
        client_request_id: "actual-generated-retrieval".into(),
        paper_id: Some("paper".into()),
        query: "Read this paper Ω".into(),
    }
}
async fn fixture() -> Fixture {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    paper_preparation::seed(&f);
    f.bind_actual_package(catalog, key);
    seed(&f).await;
    f
}
fn counts(f: &Fixture) -> (i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from concept_retrieval_traces),(select count(*) from embedding_generation_leases where owner_kind='query')", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
}

#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_retains_prepared_generation_and_lexical_ids_across_provider_wait() {
    let f = fixture().await;
    let gate = Arc::new(EmbeddingGate::default());
    let access = embedding(
        &f,
        SyntheticEmbedding {
            gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    let work = tokio::spawn(commands::preview_selected(
        f.store.clone(),
        access,
        ConceptRetrievalManager::default(),
        draft(),
    ));
    tokio::time::timeout(WAIT, gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(counts(&f), (0, 1));
    f.store.connection().unwrap().execute_batch("update embedding_index_generations set status='superseded',superseded_at='now' where id='retrieval-generation'; update ai_provider_settings set model_id='later-model' where provider_id='qwen-embedding'; update concepts set label='Changed identity', explanation='unrelated' where id='retrieval-concept';").unwrap();
    gate.release.notify_one();
    let preview = tokio::time::timeout(WAIT, work)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        preview.index_generation_id.as_deref(),
        Some("retrieval-generation")
    );
    assert_eq!(preview.seeds[0].label, "Changed identity");
    assert_eq!(preview.seeds[0].matched_by, ["lexical", "vector"]);
    assert_eq!(counts(&f), (1, 0));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_cancel_after_receiver_drop_releases_lease_and_manager_slot() {
    let f = fixture().await;
    let gate = Arc::new(EmbeddingGate::default());
    let manager = ConceptRetrievalManager::default();
    let access = embedding(
        &f,
        SyntheticEmbedding {
            gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    let receiver = tokio::spawn(commands::preview_selected(
        f.store.clone(),
        access,
        manager.clone(),
        draft(),
    ));
    tokio::time::timeout(WAIT, gate.entered.notified())
        .await
        .unwrap();
    receiver.abort();
    assert!(receiver.await.unwrap_err().is_cancelled());
    assert!(manager.cancel(&draft().client_request_id).unwrap());
    tokio::time::timeout(WAIT, async {
        while manager.cancel(&draft().client_request_id).unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(counts(&f), (0, 0));
    let registration = manager.register_owned(&draft().client_request_id).unwrap();
    drop(registration);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_provider_error_preserves_safe_cause_after_release() {
    let f = fixture().await;
    let access = embedding(
        &f,
        SyntheticEmbedding {
            rate_limited: true,
            ..Default::default()
        },
    );
    let error = commands::preview_selected(
        f.store.clone(),
        access,
        ConceptRetrievalManager::default(),
        draft(),
    )
    .await
    .unwrap_err();
    assert_eq!(error["code"], "provider_unavailable");
    assert_eq!(
        error["providerError"],
        json!({"code":"rate_limited","retryAfterMs":5000})
    );
    assert_eq!(error["leaseReleaseConfirmed"], true);
    assert_eq!(error["traceWriteStarted"], false);
    assert!(!error.to_string().contains("untrusted synthetic body"));
    assert_eq!(counts(&f), (0, 0));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_release_sql_failure_precedes_provider_error() {
    let f = fixture().await;
    let gate = Arc::new(EmbeddingGate::default());
    let access = embedding(
        &f,
        SyntheticEmbedding {
            gate: Some(gate.clone()),
            rate_limited: true,
        },
    );
    let work = tokio::spawn(commands::preview_selected(
        f.store.clone(),
        access,
        ConceptRetrievalManager::default(),
        draft(),
    ));
    tokio::time::timeout(WAIT, gate.entered.notified())
        .await
        .unwrap();
    f.store.connection().unwrap().execute_batch("create temp trigger reject_release before delete on embedding_generation_leases begin select raise(abort,'synthetic release failure'); end;").unwrap();
    gate.release.notify_one();
    let error = tokio::time::timeout(WAIT, work)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error["code"], "storage_error");
    assert_eq!(error["nativeFailure"]["kind"], "sourceData");
    assert_eq!(error["leaseReleaseConfirmed"], false);
    assert!(error.get("providerError").is_none());
    assert_eq!(counts(&f), (0, 1));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_trace_insert_failure_rolls_back_after_lease_release() {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger reject_trace before insert on concept_retrieval_traces begin select raise(abort,'synthetic trace failure'); end;").unwrap();
    let access = embedding(&f, SyntheticEmbedding::default());
    let error = commands::preview_selected(
        f.store.clone(),
        access,
        ConceptRetrievalManager::default(),
        draft(),
    )
    .await
    .unwrap_err();
    assert_eq!(error["code"], "storage_error");
    assert_eq!(error["nativeFailure"]["outcome"], "rolledBack");
    assert_eq!(error["leaseReleaseConfirmed"], true);
    assert_eq!(error["traceWriteStarted"], true);
    assert_eq!(counts(&f), (0, 0));
    f.service.shutdown().await.unwrap();
}

pub(super) fn context(f: &Fixture, trace: &str) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select context_text from concept_retrieval_traces where id=?1",
            [trace],
            |row| row.get(0),
        )
        .unwrap()
}

async fn seed(f: &Fixture) {
    {
        let db = f.store.connection().unwrap();
        db.execute_batch("update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4',base_url='https://dashscope.aliyuncs.com/compatible-mode/v1',timeout_ms=30000 where provider_id='qwen-embedding';
            insert into concepts(id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at) values('retrieval-concept','Read this paper Ω','Synthetic retrieval identity','','','','','human_original',1,'now','now');
            insert into embedding_spaces(id,provider_id,region,model_id,model_revision,dimension,scalar_type,distance_metric,normalization,document_input_role,query_input_role,instruction_set_version,created_at) values('retrieval-space','qwen-embedding','cn-beijing','text-embedding-v4','text-embedding-v4',3,'float32','cosine','none','document','query','v1','now');
            insert into semantic_representations(id,subject_kind,subject_id,subject_revision,representation_kind,namespace,language,render_version,rendered_text,input_sha256,created_at) values('retrieval-representation','concept','retrieval-concept',1,'concept_identity','accepted_concepts','en','v1','Read this paper Ω','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now');
            insert into embedding_index_generations(id,namespace,embedding_space_id,backend_kind,generation,status,relative_database_path,lance_table,index_kind,config_json,created_at,activated_at) values('retrieval-generation','accepted_concepts','retrieval-space','lancedb',1,'active','retrieval-space/generation-000001','accepted_concepts','exact','{}','now','now');
            insert into embedding_records(id,representation_id,embedding_space_id,index_generation_id,lance_table,lance_row_id,vector_encoding,status,created_at,updated_at) values('retrieval-record','retrieval-representation','retrieval-space','retrieval-generation','accepted_concepts','retrieval-row','float32','queued','now','now');
            insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) select 'retrieval-anchor',item_id,'selected',content_sha256,0,'Original retrieval evidence Ω','now' from attachments where id='selected';
            insert into definition_claims(id,concept_id,statement,created_at,updated_at) values('retrieval-definition','retrieval-concept','Evidence definition Ω','now','now');
            insert into definition_claim_evidence_links(id,definition_claim_id,anchor_id,stance,created_at) values('retrieval-link','retrieval-definition','retrieval-anchor','supports','now');").unwrap();
    }
    let vectors = VectorAccess::new(f.store.semantic_index_directory().to_path_buf())
        .open(
            "retrieval-space/generation-000001",
            SemanticNamespace::AcceptedConcepts,
            3,
        )
        .await
        .unwrap();
    vectors.ensure_ready().await.unwrap();
    let receipt = vectors
        .upsert(&SemanticVectorRow {
            row_id: "retrieval-row".into(),
            representation_id: "retrieval-representation".into(),
            subject_kind: "concept".into(),
            subject_id: "retrieval-concept".into(),
            subject_revision: 1,
            representation_kind: "concept_identity".into(),
            language: "en".into(),
            input_sha256: "a".repeat(64),
            embedding_space_id: "retrieval-space".into(),
            generation: 1,
            vector: vec![1.0, 0.0, 0.0],
            created_at: "now".into(),
        })
        .await
        .unwrap();
    f.store.connection().unwrap().execute(
        "update embedding_records set status='ready',lance_dataset_version=?1,vector_norm=1.0 where id='retrieval-record'",
        [i64::try_from(receipt.dataset_version).unwrap()],
    ).unwrap();
}
