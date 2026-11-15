use super::*;
use crate::ai::embedding_settings::AiProviderSettingsDraft;

fn draft() -> AiProviderSettingsDraft {
    AiProviderSettingsDraft {
        provider_id: "qwen-embedding".into(),
        enabled: true,
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
        region: "cn-beijing".into(),
        model_id: "text-embedding-v4".into(),
        thinking_mode: "disabled".into(),
        timeout_ms: 30_000,
    }
}
fn save(f: &Fixture) -> Result<crate::ai::embedding_settings::AiProviderSettings, Value> {
    let credentials = JsonCredentialStore::from_path(f._data.path().join("synthetic-index.json"));
    crate::model_settings_app::save_selected(&f.store, &credentials, draft())
}
fn count(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from embedding_outbox_jobs", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_provider_save_backfills_once_then_original_job_becomes_ready() {
    let (f, _, embedding) = fixture(false, IndexEmbedding::default()).await;
    assert_eq!(count(&f), 0);
    assert!(save(&f).unwrap().enabled);
    assert_eq!(count(&f), 1);
    let (_, job) = job_info(&f);
    assert!(save(&f).unwrap().enabled);
    assert_eq!(count(&f), 1);
    assert_eq!(job_info(&f).1, job);
    let summary = start(&f, embedding, SemanticIndexManager::default()).await;
    assert_eq!(summary.job_id, job);
    assert_eq!(wait_job(&f, &job).await.status, "succeeded");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_backfill_failure_keeps_original_saved_settings_and_retries() {
    let (f, _, _) = fixture(false, IndexEmbedding::default()).await;
    f.store.connection().unwrap().execute_batch("create temp trigger deny_backfill before insert on embedding_spaces begin select raise(abort,'synthetic backfill failure'); end;").unwrap();
    let failed = save(&f).unwrap_err();
    assert_eq!(failed["code"], "storage_error");
    assert_eq!(failed["indexOperation"], "provider-backfill");
    assert!(!failed.to_string().contains("synthetic backfill failure"));
    assert_eq!(count(&f), 0);
    let enabled: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select enabled from ai_provider_settings where provider_id='qwen-embedding'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        enabled,
        "original settings save precedes non-transactional backfill"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger deny_backfill")
        .unwrap();
    assert!(save(&f).unwrap().enabled);
    assert_eq!(count(&f), 1);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_ready_index_and_original_job_survive_core_reopen_and_retrieval() {
    let (mut f, concept, embedding) = fixture(true, IndexEmbedding::default()).await;
    let summary = start(&f, embedding.clone(), SemanticIndexManager::default()).await;
    let ready = wait_job(&f, &summary.job_id).await;
    assert_eq!(ready.status, "succeeded");
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: f._data.path().into(),
            agent_configuration_directory: f._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    assert!(f.store.semantic_index_source().is_none());
    assert_eq!(
        serde_json::to_value(wait_job(&f, &summary.job_id).await).unwrap(),
        serde_json::to_value(ready).unwrap()
    );
    let preview = commands::preview_selected(
        f.store.clone(),
        embedding,
        ConceptRetrievalManager::default(),
        ConceptRetrievalPreviewDraft {
            client_request_id: "index-after-reopen".into(),
            paper_id: Some("paper".into()),
            query: "Index concept".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(preview.semantic_status, "used");
    assert!(preview.seeds.iter().any(|seed| seed.concept_id == concept
        && seed.matched_by.iter().any(|matched| matched == "vector")));
    f.service.shutdown().await.unwrap();
}
