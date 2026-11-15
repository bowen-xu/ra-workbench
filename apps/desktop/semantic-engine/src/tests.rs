use research_semantic_engine_api::{SemanticNamespace, SemanticVectorRow};

use crate::store::LanceDbStore;

fn row(id: &str, vector: Vec<f32>) -> SemanticVectorRow {
    SemanticVectorRow {
        row_id: id.to_owned(),
        representation_id: format!("representation-{id}"),
        subject_kind: "concept".to_owned(),
        subject_id: format!("concept-{id}"),
        subject_revision: 1,
        representation_kind: "concept_identity".to_owned(),
        language: "en".to_owned(),
        input_sha256: "a".repeat(64),
        embedding_space_id: "space-1".to_owned(),
        generation: 1,
        vector,
        created_at: "2026-08-11T00:00:00.000Z".to_owned(),
    }
}

#[tokio::test]
async fn creates_upserts_searches_reopens_and_deletes_local_table() {
    let directory = tempfile::tempdir().unwrap();
    let store = LanceDbStore::open(directory.path(), SemanticNamespace::AcceptedConcepts, 3)
        .await
        .unwrap();
    let first = store
        .upsert(&row("row-a", vec![1.0, 0.0, 0.0]))
        .await
        .unwrap();
    let second = store
        .upsert(&row("row-b", vec![0.0, 1.0, 0.0]))
        .await
        .unwrap();
    assert!(second.dataset_version >= first.dataset_version);
    let update = store
        .upsert(&row("row-b", vec![0.9, 0.1, 0.0]))
        .await
        .unwrap();
    assert_eq!(update.updated_rows, 1);

    let hits = store.search(&[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].row_id, "row-a");
    assert!(hits[0].distance <= hits[1].distance);
    assert!(store.contains("row-a", &"a".repeat(64)).await.unwrap());

    drop(store);
    let reopened = LanceDbStore::open(directory.path(), SemanticNamespace::AcceptedConcepts, 3)
        .await
        .unwrap();
    assert!(reopened.ensure_ready().await.unwrap() >= second.dataset_version);
    let deleted_version = reopened.delete("row-a").await.unwrap();
    assert!(deleted_version > second.dataset_version);
    let remaining = reopened.search(&[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].row_id, "row-b");
}

#[tokio::test]
async fn rejects_wrong_dimensions_non_finite_values_and_namespace_leaks() {
    let directory = tempfile::tempdir().unwrap();
    let accepted = LanceDbStore::open(directory.path(), SemanticNamespace::AcceptedConcepts, 3)
        .await
        .unwrap();
    assert!(accepted.upsert(&row("short", vec![1.0])).await.is_err());
    assert!(
        accepted
            .upsert(&row("nan", vec![f32::NAN, 0.0, 0.0]))
            .await
            .is_err()
    );
    accepted
        .upsert(&row("accepted", vec![1.0, 0.0, 0.0]))
        .await
        .unwrap();

    let staging = LanceDbStore::open(directory.path(), SemanticNamespace::CandidateStaging, 3)
        .await
        .unwrap();
    assert!(
        staging
            .search(&[1.0, 0.0, 0.0], 2)
            .await
            .unwrap()
            .is_empty()
    );
}
