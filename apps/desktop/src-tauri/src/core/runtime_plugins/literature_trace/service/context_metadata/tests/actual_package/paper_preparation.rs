//! Original preparation entries on an independent package and Core-owned objects.
use super::*;
use crate::{
    ai::{
        paper_context::prepare,
        paper_reading_contracts::{PaperAiReadingImageAssetDraft, PrepareRequest},
        reading_support::PaperReadingImages,
    },
    docling::preparation::test_support::attachment,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

pub(super) fn expected_preview(
    db: &rusqlite::Connection,
    request: &crate::ai::paper_reading_contracts::PaperContextRequest,
) -> crate::ai::paper_reading_contracts::PaperContextPreview {
    let (catalog, _) = actual_package();
    let package = catalog.discover().unwrap().packages.remove(0);
    crate::runtime_plugins::literature_context::test_fixture::expected_preview(
        &catalog, &package, db, request,
    )
}
pub(super) fn seed_snapshot(
    db: &mut rusqlite::Connection,
    paper: &str,
    prepared: &crate::ai::paper_reading_contracts::PaperAiReadingPrepareResult,
    image_hashes: std::collections::BTreeMap<String, String>,
) -> crate::ai::paper_reading_contracts::PaperContextSnapshot {
    let (catalog, _) = actual_package();
    let package = catalog.discover().unwrap().packages.remove(0);
    crate::runtime_plugins::literature_context::test_fixture::seed_snapshot(
        &catalog,
        &package,
        db,
        paper,
        prepared,
        image_hashes,
    )
}
pub(in super::super) fn seed(fixture: &Fixture) {
    seed_for_paper(fixture, "paper");
}
pub(super) fn seed_for_paper(fixture: &Fixture, paper: &str) {
    attachment(&fixture.store, paper, "selected", &"a".repeat(64));
    let raw = json!({
        "pages":{"1":{"page_no":1,"size":{"width":200.0,"height":400.0}}},
        "texts":[{"self_ref":"#/texts/0","text":"Prepared evidence Ω","prov":[{"page_no":1,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]}],
        "tables":[{"self_ref":"#/tables/0","data":null,"prov":[{"page_no":1,"bbox":{"l":20.0,"t":100.0,"r":120.0,"b":140.0}}]}],
        "pictures":[{"self_ref":"#/pictures/0","label":"figure","prov":[{"page_no":1,"bbox":{"l":20.0,"t":160.0,"r":120.0,"b":240.0}}]}]
    });
    fixture.store.connection().unwrap().execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values('prepare-parse',?1,'selected',?2,'fixture','success','success',?3,'{}','now','now')", rusqlite::params![paper,"a".repeat(64), raw.to_string()]).unwrap();
}
fn draft(id: &str, bytes: &[u8]) -> PaperAiReadingImageAssetDraft {
    PaperAiReadingImageAssetDraft {
        context_item_id: id.into(),
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    }
}
fn request(images: Vec<PaperAiReadingImageAssetDraft>) -> PrepareRequest {
    PrepareRequest {
        paper_id: "paper".into(),
        attachment_ids: vec!["selected".into()],
        image_assets: images,
    }
}
fn counts(fixture: &Fixture) -> (i64, i64) {
    fixture.store.connection().unwrap().query_row("select (select count(*) from paper_context_snapshots),(select count(*) from content_objects where relative_path like 'ai-reading-images/%')", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires an actual independent literature package with preparation capability"]
async fn actual_package_prepares_legacy_hint_images_snapshot_and_preview_without_static_source() {
    let (catalog, key) = actual_package();
    let mut fixture = Fixture::new();
    seed(&fixture);
    {
        let db = fixture.store.connection().unwrap();
        db.execute_batch("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('hint-provider',0,'',60000,'now'),('old-provider',0,'',60000,'now'); delete from llm_model_routes where route_kind='use_case' and route_id='paper_reading_chat'; pragma ignore_check_constraints=on;").unwrap();
        db.execute("insert into llm_model_routes(route_kind,route_id,model_kind,provider_id,model_id,reasoning_effort,updated_at) values('default','vision','vision','hint-provider','hint-model','high','now') on conflict(route_kind,route_id) do update set provider_id=excluded.provider_id,model_id=excluded.model_id,reasoning_effort=excluded.reasoning_effort", []).unwrap();
        db.execute("insert into paper_ai_reading_sessions(id,paper_id,context_snapshot_id,attachment_ids_json,parse_run_ids_json,provider_id,model_id,reasoning_effort,bootstrap_kind,source_workflow_id,source_workflow_revision,bootstrap_messages_json,bootstrap_hash,concept_retrieval_default,title,status,created_at,updated_at) values('legacy-session','paper',null,'broken legacy JSON','[\"prepare-parse\"]','old-provider','old-model',null,'fresh_paper',null,null,'[]',null,1,'旧提示 Ω','active','old','new')", []).unwrap();
    }
    fixture
        .store
        .connection()
        .unwrap()
        .execute_batch("pragma ignore_check_constraints=off;")
        .unwrap();
    let expected = {
        use crate::ai::reading_support::PaperReadingFacts;
        use crate::runtime_plugins::literature_context::preparation::PreparationResources;
        use research_workbench_native_sdk::paper_context_prepare::PaperPreparationDecisionV1;
        let db = fixture.store.connection().unwrap();
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = PreparationResources::load(&catalog, &package).unwrap();
        let capture = resources.capture_session(&db, "paper").unwrap();
        let session = match capture.input {
            Some(input) => match protocol::context_prepare(input) {
                PaperPreparationDecisionV1::Session { session } => Some(*session),
                decision => panic!("unexpected session decision: {decision:?}"),
            },
            None => None,
        };
        let facts = PaperReadingFacts::new(&db);
        let model = facts
            .selection_for_route("use_case", "paper_reading_chat")
            .unwrap()
            .or_else(|| {
                facts
                    .selection_for_route("default", "vision")
                    .ok()
                    .flatten()
            });
        prepared_context(&db, "paper", &["selected".into()])
            .unwrap()
            .into_reading_result(session, model)
    };
    let image_id = expected.image_requests[0].context_item_id.clone();
    fixture.bind_actual_package(catalog.clone(), key.clone());
    assert!(fixture.store.paper_reading_source().is_none());
    let prepared =
        prepare::prepare_reading_context(&fixture.store, "paper".into(), vec!["selected".into()])
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&prepared).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(prepared.model.as_ref().unwrap().model_id, "hint-model");
    assert!(prepared.session.as_ref().unwrap().attachment_ids.is_empty());
    assert_eq!(
        prepared
            .context_items
            .iter()
            .find(|item| item.kind == "table")
            .unwrap()
            .table_json,
        Some(Value::Null)
    );
    assert_eq!(counts(&fixture), (0, 0));
    // Empty screenshot input retains the original permitted snapshot behavior.
    let empty = prepare::prepare_context(&fixture.store, request(vec![]))
        .await
        .unwrap();
    assert_eq!(counts(&fixture), (1, 0));
    let png = b"\x89PNG\r\n\x1a\ncontrolled-preparation-image";
    let image = draft(&image_id, png);
    let snapshot = prepare::prepare_context(&fixture.store, request(vec![image]))
        .await
        .unwrap();
    assert_ne!(snapshot.context_hash, empty.context_hash);
    assert_eq!(counts(&fixture), (2, 1));
    let hash = format!("{:x}", Sha256::digest(png));
    {
        let db = fixture.store.connection().unwrap();
        assert_eq!(
            PaperReadingImages::new(&db, &fixture.store)
                .read(&hash)
                .unwrap()
                .bytes,
            png
        );
    }
    let preview_request = crate::ai::paper_reading_contracts::PaperContextRequest {
        paper_id: "paper".into(),
        snapshot_id: snapshot.snapshot_id.clone(),
        expected_context_hash: snapshot.context_hash.clone(),
    };
    let oracle = expected_preview(&fixture.store.connection().unwrap(), &preview_request);
    let preview =
        crate::ai::paper_context::preview::preview_context(&fixture.store, preview_request)
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&preview).unwrap(),
        serde_json::to_vec(&oracle).unwrap()
    );
    assert_eq!(
        preview
            .items
            .iter()
            .find(|item| item.item.kind == "picture")
            .unwrap()
            .item
            .image_content_sha256
            .as_deref(),
        Some(hash.as_str())
    );
    let repeated = prepare::prepare_context(&fixture.store, request(vec![draft(&image_id, png)]))
        .await
        .unwrap();
    assert_eq!(repeated.snapshot_id, snapshot.snapshot_id);
    assert_eq!(counts(&fixture), (2, 1));
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
    fixture.bind_actual_package(catalog, key);
    let reopened = prepare::prepare_context(&fixture.store, request(vec![draft(&image_id, png)]))
        .await
        .unwrap();
    assert_eq!(reopened.snapshot_id, snapshot.snapshot_id);
    assert_eq!(counts(&fixture), (2, 1));
    assert_eq!(fixture.proofs(), 0);
    let after_reopen =
        prepare::prepare_reading_context(&fixture.store, "paper".into(), vec!["selected".into()])
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&after_reopen).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an actual independent literature package with preparation capability"]
async fn actual_package_keeps_first_image_when_later_duplicate_fails_and_retries_same_snapshot() {
    let (catalog, key) = actual_package();
    let mut fixture = Fixture::new();
    seed(&fixture);
    fixture.bind_actual_package(catalog, key);
    let prepared =
        prepare::prepare_reading_context(&fixture.store, "paper".into(), vec!["selected".into()])
            .await
            .unwrap();
    let id = &prepared.image_requests[0].context_item_id;
    let png = b"\x89PNG\r\n\x1a\ncontrolled-partial-image";
    let mut duplicate = draft(id, png);
    duplicate.data_url = "invalid-second-image".into();
    let error = prepare::prepare_context(&fixture.store, request(vec![draft(id, png), duplicate]))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "invalidInput", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("图片截图包含重复结构化块 ID")
    );
    assert_eq!(error["savedImageAssetCount"], 1);
    assert_eq!(error["imageObjectWriteStarted"], true);
    assert_eq!(counts(&fixture), (0, 1));
    let complete = prepare::prepare_context(&fixture.store, request(vec![draft(id, png)]))
        .await
        .unwrap();
    assert_eq!(counts(&fixture), (1, 1));
    {
        let db = fixture.store.connection().unwrap();
        let next = "b".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'application/pdf',1,'now')", [&next]).unwrap();
        db.execute(
            "update attachments set content_sha256=?1 where id='selected'",
            [&next],
        )
        .unwrap();
    }
    let mut bad = draft(id, png);
    bad.data_url = "invalid-image-on-stale-parse".into();
    let error = prepare::prepare_context(&fixture.store, request(vec![bad]))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");
    assert_eq!(counts(&fixture), (1, 1));
    assert!(error.get("imageObjectWriteStarted").is_none());
    assert!(!complete.snapshot_id.is_empty());
    fixture.service.shutdown().await.unwrap();
}
