use super::*;
use research_agent_core::submission::SubmissionStore;
use research_workbench_native_sdk::catalog::CatalogOutput;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn original_registration_reads_current_metadata_with_original_source_identity_and_pagination()
{
    let mut fixture = Fixture::new();
    let output = fixture.response();
    fixture.configure(&[output.clone(), output], None, false);
    let saved =
        crate::agent::repository::submissions::SqliteSubmissionStore::for_app(&fixture.store)
            .get(fixture.record.conversation_id, fixture.record.request_id)
            .await
            .unwrap();
    assert_eq!(saved.input, fixture.record.input);
    assert!(fixture.store.paper_metadata_source().is_none());
    let request = fixture.metadata_read();
    let first = fixture.execute(request.clone()).await;
    assert!(!first.is_error, "{}", first.content);
    let first: Value = serde_json::from_str(&first.content).unwrap();
    let text = first["text"].as_str().unwrap();
    let body: Value = serde_json::from_str(text).unwrap();
    assert_eq!(body["title"], "Metadata Ω");
    assert_eq!(body["creators"], json!([["Family Given ", "author"]]));
    assert_eq!(first["sourceId"], request.arguments["sourceId"]);
    assert_eq!(first["turnId"], fixture.record.turn_id.to_string());
    let mut page = request;
    page.arguments["startChar"] = json!(3);
    page.arguments["maxChars"] = json!(17);
    let page = fixture.execute(page).await;
    assert!(!page.is_error, "{}", page.content);
    let page: Value = serde_json::from_str(&page.content).unwrap();
    assert_eq!(page["readViewId"], first["readViewId"]);
    assert_eq!(
        page["text"],
        text.chars().skip(3).take(17).collect::<String>()
    );
    assert_eq!(page["nextChar"], 20);
    assert_eq!(
        fixture.proofs(),
        0,
        "metadata must not invent a durable read proof"
    );
    fixture.service.shutdown().await.unwrap();
    assert_eq!(fixture.calls(), 2);
}

#[tokio::test]
async fn mixed_search_preserves_note_metadata_portrait_order_and_confirmed_note_proof_upsert() {
    let mut fixture = Fixture::new();
    fixture.mixed(true);
    let response = fixture.response();
    fixture.configure(&[response.clone(), response], None, false);
    let first = fixture.execute(fixture.search()).await;
    assert!(!first.is_error, "{}", first.content);
    let first: Value = serde_json::from_str(&first.content).unwrap();
    let matches = first["matches"].as_array().unwrap();
    let note_source = &fixture.record.input.context[0].source_id;
    let metadata_source = &fixture.record.input.context[1].source_id;
    assert_eq!(matches.first().unwrap()["sourceId"], *note_source);
    let metadata_position = matches
        .iter()
        .position(|value| value["sourceId"] == *metadata_source)
        .unwrap();
    let portrait_position = matches
        .iter()
        .position(|value| {
            value["sourceUrl"]
                .as_str()
                .is_some_and(|url| url.contains(&fixture.portrait))
        })
        .unwrap();
    assert!(metadata_position > 0 && portrait_position > metadata_position);
    assert_eq!(
        fixture.proofs(),
        2,
        "only the existing Note and structured-object proofs exist"
    );
    let old_proof = note_proof(&fixture);
    fixture.change_note("Changed Note Ω");
    let second = fixture.execute(fixture.search()).await;
    assert!(!second.is_error, "{}", second.content);
    assert!(second.content.contains("Changed Note Ω"));
    assert_eq!(fixture.proofs(), 2);
    assert_ne!(note_proof(&fixture), old_proof);
    fixture.service.shutdown().await.unwrap();
    assert_eq!(fixture.calls(), 2);
}

#[tokio::test]
async fn unavailable_metadata_owner_never_preempts_an_earlier_note_error_or_search_budget() {
    for mode in ["missing-binding", "missing-capability", "stopping"] {
        for budget in [false, true] {
            let mut fixture = Fixture::new();
            fixture.mixed(true);
            if budget {
                fixture.budget_notes();
            } else {
                fixture
                    .store
                    .connection()
                    .unwrap()
                    .execute(
                        "update note_blocks set content_json='\"invalid-note-content\"' where page_id=?1",
                        [&fixture.note],
                    )
                    .unwrap();
            }
            if mode != "missing-binding" {
                fixture.configure(&[], None, false);
                if mode == "missing-capability" {
                    let file = fixture.package.path().join("writing/manifest.json");
                    let mut manifest: Value =
                        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                    manifest["provides"].as_array_mut().unwrap().pop();
                    std::fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
                    fixture.bind();
                } else {
                    fixture.service.stopping.store(true, Ordering::Release);
                }
            }
            let result = fixture.execute(fixture.search()).await;
            assert!(result.is_error, "{mode}/{budget}: {}", result.content);
            let error: Value = serde_json::from_str(&result.content).unwrap();
            assert_eq!(
                error["error"],
                if budget {
                    "selected context exceeds budget"
                } else {
                    "selected source unavailable or changed"
                },
                "{mode}/{budget}"
            );
            assert!(
                error.get("nativeFailure").is_none(),
                "{mode}/{budget}: {}",
                result.content
            );
            assert!(fixture.lifecycle().is_empty());
            // Both successful Core Note reads precede the aggregate budget check.
            assert_eq!(fixture.proofs(), if budget { 2 } else { 0 });
            fixture.service.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn later_metadata_projection_error_keeps_the_already_confirmed_note_proof() {
    let mut fixture = Fixture::new();
    fixture.mixed(true);
    let mut wrong = fixture.response();
    let CatalogOutput::ContextMetadata { outcome } = &mut wrong else {
        panic!("metadata output")
    };
    outcome.input.cbor_base64 = "different-input".into();
    fixture.configure(&[wrong], None, false);
    let output = fixture.execute(fixture.search()).await;
    assert!(output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(
        value["nativeFailure"]["code"],
        "plugin_host_response_mismatch"
    );
    assert_eq!(
        fixture.proofs(),
        1,
        "the original confirmed Note upsert preceded metadata"
    );
    assert!(!note_proof(&fixture).is_empty());
    fixture.service.shutdown().await.unwrap();
}

fn note_proof(fixture: &Fixture) -> String {
    fixture.store.connection().unwrap().query_row(
        "select snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 and path=?3",
        rusqlite::params![fixture.record.conversation_id.to_string(),fixture.record.request_id.to_string(),format!("source://notes/{}",fixture.note)],
        |row|row.get(0),
    ).unwrap()
}

#[tokio::test]
async fn reaching_metadata_requires_its_actual_owner_without_a_static_fallback_or_stale_request_id()
{
    for (mode, code) in [
        ("missing-binding", "plugin_host_package_unavailable"),
        ("missing-capability", "plugin_host_capability_unavailable"),
        ("stopping", "plugin_host_stopping"),
    ] {
        let mut fixture = Fixture::new();
        if mode != "missing-binding" {
            fixture.configure(&[], None, false);
            if mode == "missing-capability" {
                let file = fixture.package.path().join("writing/manifest.json");
                let mut manifest: Value =
                    serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                manifest["provides"].as_array_mut().unwrap().pop();
                std::fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
                fixture.bind();
            } else {
                fixture.service.stopping.store(true, Ordering::Release);
            }
        }
        let output = fixture.execute(fixture.metadata_read()).await;
        assert!(output.is_error, "{mode}: {}", output.content);
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(value["nativeFailure"]["code"], code);
        assert!(value["nativeFailure"]["nativeRequestId"].is_null());
        assert_eq!(fixture.proofs(), 0);
        assert!(fixture.lifecycle().is_empty());
        fixture.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn missing_metadata_resource_retains_identity_without_reusing_an_old_native_request() {
    let mut fixture = Fixture::new();
    fixture.configure(&[fixture.response()], None, false);
    let first = fixture.execute(fixture.metadata_read()).await;
    assert!(!first.is_error, "{}", first.content);
    assert_eq!(fixture.calls(), 1);
    let accepted = fixture
        .service
        .catalog
        .discover()
        .unwrap()
        .packages
        .remove(0);
    let resource = fixture
        .package
        .path()
        .join("writing/data/context-metadata-creators.v1.sql");
    std::fs::remove_file(resource).unwrap();
    // A fixed-resource failure after package admission retains its exact cause.
    // This captures the real Catalog error without a production race/test hook.
    let error = ContextMetadataResources::load(&fixture.service.catalog, &accepted)
        .err()
        .expect("the previously accepted creators resource is now missing");
    let output = metadata_native_error(metadata_resource_error(error, &fixture.key), "admission")
        .into_output(&fixture.record, &fixture.metadata_read());
    let value: Value = serde_json::from_str(&output.content).unwrap();
    let failure = &value["nativeFailure"];
    assert_eq!(failure["code"], "runtime_plugin_resource_missing");
    assert_eq!(failure["packageKey"], fixture.key);
    assert_eq!(failure["phase"], "admission");
    assert!(failure["nativeRequestId"].is_null());
    let data = &failure["dataError"];
    assert_eq!(data["code"], "plugin_data_resource_unavailable");
    assert_eq!(data["resourceId"], "literature.context-metadata.creators");
    assert_eq!(data["resourceVersion"], 1);
    assert_eq!(data["safeCause"], "runtime_plugin_resource_missing");
    // A later whole tool call performs discovery first, so the changed package
    // is rejected before resource admission and must not invent a resource ID.
    let output = fixture.execute(fixture.metadata_read()).await;
    assert!(output.is_error, "{}", output.content);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(
        value["nativeFailure"]["code"],
        "plugin_host_package_unavailable"
    );
    assert!(value["nativeFailure"]["dataError"].is_null());
    assert!(value["nativeFailure"]["nativeRequestId"].is_null());
    assert_eq!(fixture.calls(), 1);
    assert_eq!(fixture.proofs(), 0);
    fixture.service.shutdown().await.unwrap();
}
