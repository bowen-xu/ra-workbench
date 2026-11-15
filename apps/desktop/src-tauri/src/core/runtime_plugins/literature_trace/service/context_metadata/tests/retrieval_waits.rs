//! Controlled transport verifies Core wait/recheck order, independent of policy tests.
use super::*;
use crate::{
    ai::{EmbeddingAccess, JsonCredentialStore},
    runtime_plugins::literature_retrieval::{commands, manager::ConceptRetrievalManager},
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    concept_retrieval::{
        ConceptRetrievalDecisionV1 as Decision, ConceptRetrievalInputV1 as Input,
        ConceptRetrievalPreviewDraft, ConceptRetrievalWireInputV1, ConceptRetrievalWireOutcomeV1,
    },
    paper_context_wire::encode_payload,
};
use std::fs;

fn output(input: Input, decision: Decision) -> CatalogOutput {
    CatalogOutput::ConceptRetrieval {
        outcome: Box::new(ConceptRetrievalWireOutcomeV1 {
            input: Box::new(ConceptRetrievalWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            }),
            decision_cbor_base64: encode_payload(&decision).unwrap(),
        }),
    }
}
fn configure(f: &mut Fixture, draft: &ConceptRetrievalPreviewDraft, exists: bool) {
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.concept-retrieval","version":1}));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_str()
            .unwrap()
            .starts_with("concept-retrieval-")
        {
            fs::copy(entry.path(), directory.join("data").join(entry.file_name())).unwrap();
        }
    }
    f.configure(
        &[
            output(
                Input::ValidateDraft {
                    draft: draft.clone(),
                },
                Decision::Query {
                    query: draft.query.clone(),
                    fts_query: "Read".into(),
                    query_sha256: "a".repeat(64),
                },
            ),
            output(
                Input::ValidatePaper { exists },
                if exists {
                    Decision::PaperValidated
                } else {
                    Decision::Rejected {
                        message: "当前论文不存在".into(),
                    }
                },
            ),
        ],
        Some(1),
        false,
    );
}

#[tokio::test]
async fn native_paper_wait_releases_database_guard_and_rechecks_absent_row_before_rejection() {
    let mut f = Fixture::new();
    let draft = ConceptRetrievalPreviewDraft {
        client_request_id: "wait-preview".into(),
        paper_id: Some("appeared".into()),
        query: "Read".into(),
    };
    configure(&mut f, &draft, false);
    let access = EmbeddingAccess::new(JsonCredentialStore::from_path(
        f._data.path().join("empty-credentials.json"),
    ));
    let work = tokio::spawn(commands::preview_selected(
        f.store.clone(),
        access,
        ConceptRetrievalManager::default(),
        draft,
    ));
    let entered = f.entered(1).await;
    let store = f.store.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('appeared','journalArticle','Appeared','now','now')", []).unwrap();
    });
    let writer_finished = tokio::time::timeout(WAIT, writer).await;
    f.release();
    assert!(entered);
    writer_finished.unwrap().unwrap();
    let error = tokio::time::timeout(WAIT, work)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error["code"], "storage_error");
    assert_eq!(
        error["nativeFailure"]["dataError"]["code"],
        "plugin_data_snapshot_changed"
    );
    assert!(error.get("providerError").is_none());
    assert!(error["nativeFailure"].get("nativeRequestId").is_none());
    assert_eq!(f.calls(), 2);
    f.service.shutdown().await.unwrap();
}
