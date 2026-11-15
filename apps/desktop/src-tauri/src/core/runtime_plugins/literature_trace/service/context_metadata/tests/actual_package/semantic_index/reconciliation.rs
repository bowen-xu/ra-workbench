use super::*;
use crate::{
    runtime_plugins::literature_index::IndexOperation,
    semantic_engine::{SemanticVectorStore, VectorAccess},
};

#[tokio::test]
#[ignore = "requires the current independent literature package and C ABI engine"]
async fn actual_package_repairs_wrong_hash_missing_vector_and_orphan_then_reindexes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (f, _, embedding) = fixture(
        true,
        IndexEmbedding {
            calls: Arc::clone(&calls),
            ..Default::default()
        },
    )
    .await;
    let first = start(&f, embedding.clone(), SemanticIndexManager::default()).await;
    assert_eq!(wait_job(&f, &first.job_id).await.status, "succeeded");
    let IndexOutput::Prepared(prepared) = f
        .service
        .semantic_index_request(
            f.store.clone(),
            f.key.clone(),
            IndexOperation::Prepare {
                id: first.job_id.clone(),
                embedding: embedding.clone(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("original prepared job");
    };
    let (job, _) = *prepared;
    let access = VectorAccess::new(f.store.semantic_index_directory().to_path_buf());
    let mut vectors = access
        .open(&job.relative_database_path, job.namespace, job.dimension)
        .await
        .unwrap();
    let mut wrong = job.row;
    wrong.vector = vec![0.0; job.dimension];
    wrong.vector[0] = 1.0;
    wrong.input_sha256 = "b".repeat(64);
    vectors.upsert(&wrong).await.unwrap();
    let mut orphan = wrong.clone();
    orphan.row_id = "synthetic-orphan".into();
    orphan.representation_id = "synthetic-orphan-representation".into();
    vectors.upsert(&orphan).await.unwrap();
    for (attempt, orphans) in [(2, 1), (3, 0)] {
        if attempt == 3 {
            // The second pass exercises a ready SQL record with a missing vector.
            vectors.delete(&wrong.row_id).await.unwrap();
        }
        // Each production operation opens its own dataset session. A retained
        // fixture session still observes the earlier Lance snapshot.
        drop(vectors);
        let summary =
            index_commands::reconcile_selected(f.store.clone(), SemanticIndexManager::default())
                .await
                .unwrap();
        assert_eq!(summary.status, "succeeded");
        assert_eq!(summary.checked_count, 1);
        assert_eq!(summary.missing_count, 1);
        assert_eq!(summary.orphan_count, orphans);
        assert_eq!(summary.repaired_count, 1 + orphans);
        vectors = access
            .open(&job.relative_database_path, job.namespace, job.dimension)
            .await
            .unwrap();
        assert!(vectors.list_metadata().await.unwrap().is_empty());
        drop(vectors);
        let queued = start(&f, embedding.clone(), SemanticIndexManager::default()).await;
        assert_eq!(queued.attempt, attempt);
        assert_eq!(wait_job(&f, &queued.job_id).await.status, "succeeded");
        vectors = access
            .open(&job.relative_database_path, job.namespace, job.dimension)
            .await
            .unwrap();
        let rows = vectors.list_metadata().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_id, wrong.row_id);
        assert_ne!(rows[0].input_sha256, wrong.input_sha256);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    f.service.shutdown().await.unwrap();
}
