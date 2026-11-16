//! The original extraction preparation, including required picture screenshots.
use super::*;
use crate::ai::{
    paper_reading_contracts::{PaperAiReadingImageAssetDraft, PrepareRequest},
    reading_support::PaperReadingImages,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_prepares_analysis_sources_images_and_snapshot_without_reading_hints() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    paper_preparation::seed(&f);
    let expected = prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap()
    .into_reading_result(None, None);
    let requested = expected
        .image_requests
        .iter()
        .map(|image| image.context_item_id.clone())
        .collect::<Vec<_>>();
    let missing =
        crate::ai::error::AiError::InvalidInput(protocol::image_set(&requested, &[]).unwrap_err());
    // Extraction has no legacy-session or model-hint query in this stage.
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "paper_ai_reading_sessions" | "llm_model_routes",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    f.bind_actual_package(catalog, key.clone());
    let request = |image_assets| PrepareRequest {
        paper_id: "paper".into(),
        attachment_ids: vec!["selected".into()],
        image_assets,
    };
    let error = f
        .service
        .prepare_analysis_context(f.store.clone(), key.clone(), request(vec![]))
        .await
        .unwrap_err();
    assert_eq!(error["code"], missing.code());
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains(&missing.to_string())
    );
    let snapshots: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(snapshots, 0);
    let image = PaperAiReadingImageAssetDraft {
        context_item_id: expected.image_requests[0].context_item_id.clone(),
        data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(b"\x89PNG\r\n\x1a\nsynthetic-analysis-image")
        ),
    };
    let (actual, snapshot) = f
        .service
        .prepare_analysis_context(f.store.clone(), key.clone(), request(vec![image.clone()]))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert!(actual.model.is_none());
    assert!(actual.session.is_none());
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let oracle = {
        let mut db = f.store.connection().unwrap();
        let hash = PaperReadingImages::new(&db, &f.store)
            .save(&image.data_url)
            .unwrap();
        protocol::image_set(&requested, std::slice::from_ref(&image.context_item_id)).unwrap();
        paper_preparation::seed_snapshot(
            &mut db,
            "paper",
            &expected,
            BTreeMap::from([(image.context_item_id.clone(), hash)]),
        )
    };
    assert_eq!(snapshot.snapshot_id, oracle.id);
    assert_eq!(snapshot.context_hash, oracle.context_hash);
    let (_, repeated) = f
        .service
        .prepare_analysis_context(f.store.clone(), key, request(vec![image]))
        .await
        .unwrap();
    assert_eq!(repeated.snapshot_id, snapshot.snapshot_id);
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    let snapshots: i64 = db
        .query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(snapshots, 1);
    drop(db);
    f.service.shutdown().await.unwrap();
}
