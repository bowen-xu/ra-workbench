//! The original concept save queues a real job. Only provider HTTP is synthetic.
use super::*;
use crate::{
    ai::{EmbeddingAccess, EmbeddingHttpWire, JsonCredentialStore, ProviderError},
    runtime_plugins::{
        literature_concepts::{ConceptRequest, ConceptResult},
        literature_index::{
            IndexOutput, commands as index_commands, manager::SemanticIndexManager,
        },
        literature_retrieval::{commands, manager::ConceptRetrievalManager},
    },
};
use research_workbench_native_sdk::semantic_index::EmbeddingJobSummary;
mod backfill;
mod lifecycle;
mod reconciliation;
use research_workbench_native_sdk::{
    concept_retrieval::ConceptRetrievalPreviewDraft, concepts::ConceptDraft,
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct IndexEmbedding {
    gate: Option<Arc<IndexGate>>,
    rate_limited: bool,
    calls: Arc<AtomicUsize>,
}
#[derive(Default)]
struct IndexGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl EmbeddingHttpWire for IndexEmbedding {
    fn respond(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = Result<reqwest::Response, ProviderError>> + Send + '_>> {
        let payload: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(request.url().scheme(), "https");
        assert_eq!(payload["model"], "text-embedding-v4");
        assert_eq!(payload["dimensions"], 1024);
        let mut vector = vec![0.0f32; 1024];
        vector[0] = 1.0;
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if let Some(gate) = &self.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if self.rate_limited {
                return Ok(hyper::Response::builder()
                    .status(429)
                    .header("retry-after", "5")
                    .body(reqwest::Body::from("untrusted synthetic failure"))
                    .unwrap()
                    .into());
            }
            Ok(hyper::Response::builder()
                .status(200)
                .body(reqwest::Body::from(
                    json!({
                        "model":"text-embedding-v4", "data":[{"index":0,"embedding":vector}],
                        "usage":{"prompt_tokens":1,"total_tokens":1}
                    })
                    .to_string(),
                ))
                .unwrap()
                .into())
        })
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_indexes_original_queued_concept_and_uses_vector_in_retrieval() {
    let (f, concept_id, embedding) = fixture(true, IndexEmbedding::default()).await;
    let (representation, original_job) = job_info(&f);
    assert!(f.store.semantic_index_source().is_none());
    let summary = index_commands::retry_selected(
        f.store.clone(),
        embedding.clone(),
        SemanticIndexManager::default(),
        representation,
    )
    .await
    .unwrap();
    assert_eq!(summary.job_id, original_job);
    assert_eq!(summary.attempt, 1);
    let finished = wait_job(&f, &original_job).await;
    assert_eq!(finished.status, "succeeded", "{finished:?}");
    let ready: (String, i64, f64) = f.store.connection().unwrap().query_row("select status,lance_dataset_version,vector_norm from embedding_records where representation_id=?1", [&finished.representation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(ready.0, "ready");
    assert!(ready.1 > 0);
    assert_eq!(ready.2, 1.0);
    assert!(f.store.retrieval_preview_source().is_none());
    let preview = commands::preview_selected(
        f.store.clone(),
        embedding,
        ConceptRetrievalManager::default(),
        ConceptRetrievalPreviewDraft {
            client_request_id: "actual-index-to-retrieval".into(),
            paper_id: Some("paper".into()),
            query: "Index concept".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(preview.semantic_status, "used");
    assert!(
        preview
            .seeds
            .iter()
            .any(|seed| seed.concept_id == concept_id
                && seed.matched_by.iter().any(|value| value == "vector"))
    );
    f.service.shutdown().await.unwrap();
    println!(
        "{}",
        json!({"acceptance":"original-concept-queue-to-index-to-retrieval","packageKey":f.key,"jobId":original_job,"datasetVersion":ready.1,"traceId":preview.trace_id,"syntheticHttp":true})
    );
}

async fn fixture(enabled: bool, wire: IndexEmbedding) -> (Fixture, String, EmbeddingAccess) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    f.store.connection().unwrap().execute("update ai_provider_settings set enabled=?1,region='cn-beijing',model_id='text-embedding-v4',base_url='https://dashscope.aliyuncs.com/compatible-mode/v1',timeout_ms=30000 where provider_id='qwen-embedding'", [enabled]).unwrap();
    let concept = f
        .service
        .concepts(
            f.store.shared_agent_connection(),
            f.key.clone(),
            ConceptRequest::Save {
                draft: Box::new(ConceptDraft {
                    id: None,
                    preferred_term: "Index concept".into(),
                    language: Some("en".into()),
                    label: "Index concept".into(),
                    explanation: "Synthetic index lifecycle Ω".into(),
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
    let ConceptResult::Concept(concept) = concept else {
        panic!("original concept save result");
    };
    let credentials = JsonCredentialStore::from_path(f._data.path().join("synthetic-index.json"));
    credentials
        .set("qwen-embedding", "synthetic-index-key")
        .unwrap();
    let embedding = EmbeddingAccess::for_test(credentials, Arc::new(wire));
    assert!(f.store.semantic_index_source().is_none());
    (f, concept.id, embedding)
}
fn job_info(f: &Fixture) -> (String, String) {
    f.store.connection().unwrap().query_row("select sr.id,j.id from semantic_representations sr join embedding_records r on r.representation_id=sr.id join embedding_outbox_jobs j on j.embedding_record_id=r.id order by j.attempt desc limit 1", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
}
async fn wait_job(f: &Fixture, id: &str) -> EmbeddingJobSummary {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let IndexOutput::Job(job) = index_commands::read_selected(
                f.store.clone(),
                index_commands::IndexRead::Job(id.into()),
            )
            .await
            .unwrap() else {
                panic!("original job result");
            };
            if !matches!(job.status.as_str(), "queued" | "running") {
                break job;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn start(
    f: &Fixture,
    embedding: EmbeddingAccess,
    manager: SemanticIndexManager,
) -> EmbeddingJobSummary {
    let (representation, _) = job_info(f);
    index_commands::retry_selected(f.store.clone(), embedding, manager, representation)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_reconciles_vector_written_before_sql_ready_without_reembedding() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            calls: Arc::clone(&calls),
            ..Default::default()
        },
    )
    .await;
    f.store.connection().unwrap().execute_batch("create temp trigger reject_ready before update of status on embedding_records when new.status='ready' begin select raise(abort,'synthetic ready failure'); end;").unwrap();
    let summary = start(&f, embedding, SemanticIndexManager::default()).await;
    let failed = wait_job(&f, &summary.job_id).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error_code.as_deref(), Some("storage_error"));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_ready")
        .unwrap();
    let repaired =
        index_commands::reconcile_selected(f.store.clone(), SemanticIndexManager::default())
            .await
            .unwrap();
    assert_eq!(repaired.status, "succeeded");
    assert_eq!(
        (
            repaired.checked_count,
            repaired.missing_count,
            repaired.orphan_count,
            repaired.repaired_count
        ),
        (1, 0, 0, 1)
    );
    let (_, latest) = job_info(&f);
    let ready = wait_job(&f, &latest).await;
    assert_eq!(ready.status, "succeeded");
    assert_eq!(ready.attempt, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let IndexOutput::Status(actual) =
        index_commands::read_selected(f.store.clone(), index_commands::IndexRead::Status)
            .await
            .unwrap()
    else {
        panic!("original status result");
    };
    let IndexOutput::Status(original) = index_commands::read_selected(f.store.clone(), index_commands::IndexRead::Status).await.unwrap() else { panic!("retained dynamic status"); };
    assert_eq!(actual.spaces.len(),1);
    assert_eq!(actual.spaces[0].ready_count,1);
    assert!(actual.running_job_ids.is_empty());
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_cancels_before_running_while_original_embedding_waits() {
    let gate = Arc::new(IndexGate::default());
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            gate: Some(Arc::clone(&gate)),
            ..Default::default()
        },
    )
    .await;
    let manager = SemanticIndexManager::default();
    let summary = start(&f, embedding, manager.clone()).await;
    tokio::time::timeout(Duration::from_secs(20), gate.entered.notified())
        .await
        .unwrap();
    let before: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status from embedding_outbox_jobs where id=?1",
            [&summary.job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, "queued", "original Embedding precedes mark-running");
    assert!(manager.cancel(&summary.job_id).unwrap());
    gate.release.notify_one();
    let canceled = wait_job(&f, &summary.job_id).await;
    assert_eq!(canceled.status, "canceled");
    assert_eq!(canceled.error_code.as_deref(), Some("canceled"));
    let record: String = f
        .store
        .connection()
        .unwrap()
        .query_row("select status from embedding_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        record, "failed",
        "original canceled job marks its record failed"
    );
    f.service.shutdown().await.unwrap();
    assert!(!manager.cancel(&summary.job_id).unwrap());
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_retains_original_rate_limited_job_code() {
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            rate_limited: true,
            ..Default::default()
        },
    )
    .await;
    let summary = start(&f, embedding, SemanticIndexManager::default()).await;
    let failed = wait_job(&f, &summary.job_id).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error_code.as_deref(), Some("rate_limited"));
    let record: (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status,error_code from embedding_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(record, ("failed".into(), "rate_limited".into()));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_rebuild_retries_pending_generation_and_keeps_active_until_ready() {
    let (f, _, embedding) = fixture(true, IndexEmbedding::default()).await;
    let summary = start(&f, embedding.clone(), SemanticIndexManager::default()).await;
    assert_eq!(wait_job(&f, &summary.job_id).await.status, "succeeded");
    let space: String = f
        .store
        .connection()
        .unwrap()
        .query_row("select id from embedding_spaces", [], |row| row.get(0))
        .unwrap();
    let IndexOutput::Generation(generation) =
        index_commands::read_selected(f.store.clone(), index_commands::IndexRead::Rebuild(space))
            .await
            .unwrap()
    else {
        panic!("original generation result");
    };
    assert_eq!(generation.status, "building");
    assert_eq!(generation.record_count, 1);
    assert_eq!(generation.ready_count, 0);
    let active: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select id from embedding_index_generations where status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let gate = Arc::new(IndexGate::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let embedding = EmbeddingAccess::for_test(
        JsonCredentialStore::from_path(f._data.path().join("synthetic-index.json")),
        Arc::new(IndexEmbedding {
            gate: Some(gate.clone()),
            calls: calls.clone(),
            ..Default::default()
        }),
    );
    let manager = SemanticIndexManager::default();
    let actual = start(&f, embedding.clone(), manager.clone()).await;
    tokio::time::timeout(Duration::from_secs(30), gate.entered.notified())
        .await
        .unwrap();
    assert_ne!(actual.job_id, summary.job_id);
    assert_eq!(actual.status, "queued");
    let pending: i64 = f.store.connection().unwrap().query_row("select count(*) from embedding_records where index_generation_id=?1 and status='queued'", [&generation.generation_id], |row| row.get(0)).unwrap();
    assert_eq!(pending, 1);
    let retained: (String, String) = f.store.connection().unwrap().query_row(
        "select g.status,r.status from embedding_index_generations g join embedding_records r on r.index_generation_id=g.id where g.id=?1",
        [&active], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(retained, ("active".into(), "ready".into()));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "rebuild starts one embedding request"
    );
    gate.release.notify_one();
    let finished = wait_job(&f, &actual.job_id).await;
    assert_eq!(finished.status, "succeeded");
    let original = index_commands::retry_selected(
        f.store.clone(),
        embedding,
        manager,
        finished.representation_id.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&finished).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    let activated: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select id from embedding_index_generations where status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(activated, generation.generation_id);
    let previous: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status from embedding_index_generations where id=?1",
            [&active],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(previous, "superseded");
    f.service.shutdown().await.unwrap();
}
