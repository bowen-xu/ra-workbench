//! Restore source geometry from real Core records using the selected Native package.
use super::*;
use crate::{
    agent::{reading_sources, repository::submissions},
    ai::{
        paper_context::{agent_preview, prepare},
        paper_reading_contracts::PrepareRequest,
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires an actual independent literature package with preview capability"]
async fn actual_package_restores_frozen_turn_geometry_without_static_source() {
    let (catalog, key) = actual_package();
    let mut fixture = Fixture::new();
    paper_preparation::seed(&fixture);
    fixture.bind_actual_package(catalog, key);
    assert!(fixture.store.paper_reading_source().is_none());
    let prepared =
        prepare::prepare_reading_context(&fixture.store, "paper".into(), vec!["selected".into()])
            .await
            .unwrap();
    let image = crate::ai::paper_reading_contracts::PaperAiReadingImageAssetDraft {
        context_item_id: prepared.image_requests[0].context_item_id.clone(),
        data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(b"\x89PNG\r\n\x1a\nturn-geometry")
        ),
    };
    let snapshot = prepare::prepare_context(
        &fixture.store,
        PrepareRequest {
            paper_id: "paper".into(),
            attachment_ids: vec!["selected".into()],
            image_assets: vec![image],
        },
    )
    .await
    .unwrap();
    let snapshot_request = agent_preview::Request {
        paper_id: "paper".into(),
        snapshot_id: snapshot.snapshot_id.clone(),
        expected_context_hash: snapshot.context_hash.clone(),
    };
    let preview =
        crate::ai::paper_context::preview::preview_context(&fixture.store, snapshot_request)
            .await
            .unwrap();
    let mut input = fixture.record.input.clone();
    input.context = agent_preview::submission_context_from_preview(preview.clone()).unwrap();
    let images = agent_preview::submission_images_from_preview(
        preview,
        &fixture.store.connection().unwrap(),
        &fixture.store,
        None,
    )
    .unwrap();
    let allowed = input
        .context
        .iter()
        .map(|source| source.source_id.clone())
        .chain(images.iter().map(|image| image.source_id.clone()))
        .collect();
    let conversation = fixture.record.conversation_id;
    let record = {
        let mut db = fixture.store.connection().unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','paper')", [conversation.to_string()]).unwrap();
        let tx = db.transaction().unwrap();
        let record =
            submissions::insert_in_with_images(&tx, conversation, Uuid::new_v4(), input, &images)
                .unwrap();
        tx.execute("insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,'paper',?4,?5)", rusqlite::params![conversation.to_string(), record.request_id.to_string(), record.turn_id.to_string(), snapshot.snapshot_id, snapshot.context_hash]).unwrap();
        tx.commit().unwrap();
        record
    };
    let package = fixture
        .service
        .catalog
        .discover()
        .unwrap()
        .packages
        .remove(0);
    let expected_preview =
        crate::runtime_plugins::literature_context::test_fixture::expected_preview(
            &fixture.service.catalog,
            &package,
            &fixture.store.connection().unwrap(),
            &agent_preview::Request {
                paper_id: "paper".into(),
                snapshot_id: snapshot.snapshot_id.clone(),
                expected_context_hash: snapshot.context_hash.clone(),
            },
        );
    agent_preview::validate_source_location_ids(&allowed).unwrap();
    let expected =
        agent_preview::source_locations_from_preview(expected_preview, &allowed).unwrap();
    let request = json!({"conversationId":conversation,"paperId":"paper","turnId":record.turn_id});
    let sources = reading_sources::read(
        &fixture.store,
        serde_json::from_value(request.clone()).unwrap(),
    )
    .await
    .unwrap();
    let sources = serde_json::to_value(sources).unwrap();
    assert_eq!(sources["sources"], serde_json::to_value(expected).unwrap());
    assert_eq!(sources["sources"].as_array().unwrap().len(), 3);
    assert_eq!(sources["retrievalTraceId"], Value::Null);
    assert_eq!(sources["turnId"], record.turn_id.to_string());
    fixture.service.shutdown().await.unwrap();
    fixture.store.shared_agent_connection().shutdown().unwrap();
    drop(fixture.store);
    fixture.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: fixture._data.path().into(),
            agent_configuration_directory: fixture._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    fixture.bind_actual_package(catalog, key);
    let reopened = reading_sources::read(&fixture.store, serde_json::from_value(request).unwrap())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(reopened).unwrap(), sources);
    fixture.service.shutdown().await.unwrap();
}
