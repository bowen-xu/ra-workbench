//! Production HTTP receiver, App binding, Core storage and an explicit compiled package.
use super::{
    protocol::{INSTANCE_HEADER, TOKEN_HEADER},
    test_support::TestServer,
};
use crate::runtime_plugins::{
    RuntimePluginCatalog,
    literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

const PDF: &[u8] = b"%PDF-1.7\n% Synthetic capture\n%%EOF\n";
fn selected() -> NoteTraceBinding {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let temporary = repository.join(".tmp").canonicalize().unwrap();
    let root = PathBuf::from(
        std::env::var_os("RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_ROOT")
            .expect("explicit package root"),
    );
    assert!(std::fs::symlink_metadata(&root).unwrap().is_dir());
    let root = root.canonicalize().unwrap();
    assert!(root.starts_with(&temporary) && root != temporary);
    let key = std::env::var("RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_KEY")
        .expect("exact package key");
    assert_eq!(key.len(), 64);
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)).with_native_writing_preview(true));
    let packages = catalog.discover().unwrap();
    let package = packages
        .packages
        .iter()
        .find(|package| package.package_key == key)
        .expect("exact package");
    assert_eq!(package.manifest["id"], "literature_library");
    let entry = package.manifest["native"]["entry"].as_str().unwrap();
    assert!(
        !catalog
            .resource(&key, entry)
            .unwrap()
            .bytes
            .starts_with(b"#!")
    );
    let service = Arc::new(LiteratureTraceService::new(catalog));
    service.browser_capture_declared(&key).unwrap();
    NoteTraceBinding {
        service,
        package_key: key,
    }
}
fn payload() -> Value {
    json!({"sessionID":"capture","uri":"https://example.org/paper","translatorID":"fixture","translatorVersion":"1", "items":[{
        "id":"parent","itemType":"journalArticle","title":"  Ω Captured Paper  ","date":"2026-11-13","DOI":"10.1234/example",
        "creators":[{"firstName":"Ada","lastName":"Example","creatorType":"author"},{"name":"Fixture Consortium","creatorType":"contributor"}],
        "tags":["Tag","TAG"],"ISBN":"978-1 234","extra":{"integer":u64::MAX,"negativeZero":-0.0,"optional":null,"text":"Ω\\\n"},"attachments":[]}]})
}
async fn save(server: &TestServer, payload: &Value) -> Value {
    let response = server.post("saveItems").json(payload).send().await.unwrap();
    assert_eq!(response.status(), 201);
    response.json().await.unwrap()
}
async fn attach(server: &TestServer, session: &str, metadata: Value) -> Value {
    let response = server
        .post(&format!("saveAttachment?sessionID={session}"))
        .header("Content-Type", "application/pdf")
        .header("X-Metadata", metadata.to_string())
        .body(PDF)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    response.json().await.unwrap()
}
fn metadata(id: &str, parent: &str) -> Value {
    json!({"id":id,"parentItemID":parent,"title":"Full Text.pdf","contentType":"application/pdf","url":"https://example.org/p.pdf"})
}

#[tokio::test]
#[ignore = "requires the current independently built literature package"]
async fn actual_package_http_capture_pdf_receipts_and_reopen_preserve_original_data() {
    let directory = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    assert!(server.shared.store.paper_metadata_source().is_none());
    let raw = payload();
    let receipt = save(&server, &raw).await;
    let id = receipt["items"][0]["itemID"].as_str().unwrap().to_owned();
    assert_eq!(receipt["items"][0]["title"], "Ω Captured Paper");
    let attachment = attach(&server, "capture", metadata("pdf", "parent")).await;
    assert_eq!(attachment["itemID"], id);
    {
        let db = server.shared.store.connection().unwrap();
        let original: String = db
            .query_row(
                "select raw_json from browser_capture_items where item_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(original, raw["items"][0].to_string());
        assert!(original.contains("\"negativeZero\":-0.0"));
        assert_eq!(
            db.query_row(
                "select count(*) from item_tags where item_id=?1",
                [&id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "select display_value from identifiers where item_id=?1 and kind='isbn'",
                [&id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "978-1 234"
        );
        db.execute(
            "update bibliographic_items set title='Manual correction' where id=?1",
            [&id],
        )
        .unwrap();
    }
    assert_eq!(save(&server, &raw).await, receipt);
    assert_eq!(
        attach(&server, "capture", metadata("pdf", "parent")).await,
        attachment
    );
    let status: Value = server
        .post("sessionStatus")
        .json(&json!({"sessionID":"capture"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["receipt"], receipt);
    assert_eq!(status["attachments"], json!([attachment.clone()]));
    server.shutdown().await;
    let reopened = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    assert_eq!(save(&reopened, &raw).await, receipt);
    assert_eq!(
        attach(&reopened, "capture", metadata("pdf", "parent")).await,
        attachment
    );
    assert_eq!(
        reopened
            .shared
            .store
            .connection()
            .unwrap()
            .query_row(
                "select title from bibliographic_items where id=?1",
                [&id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Manual correction"
    );
    reopened.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the current independently built literature package"]
async fn actual_package_http_batch_snapshot_standalone_and_session_controls_keep_original_semantics()
 {
    let directory = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    let mut batch = payload();
    batch["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":17,"itemType":"future-type","title":"Second Paper"}));
    let saved = save(&server, &batch).await;
    assert_eq!(saved["items"].as_array().unwrap().len(), 2);
    assert_eq!(saved["items"][1]["id"], 17);
    assert_eq!(saved["items"][1]["itemType"], "document");
    assert_eq!(
        saved["items"][1]["warnings"][0]["originalItemType"],
        "future-type"
    );
    let snapshot = server
        .post("saveSnapshot")
        .json(&json!({"sessionID":"web","title":"Web page","url":"https://example.org"}))
        .send()
        .await
        .unwrap();
    assert_eq!(snapshot.status(), 201);
    let direct=server.post("saveStandaloneAttachment?sessionID=standalone").header("Content-Type","application/pdf").header("X-Metadata",json!({"title":"direct.pdf","contentType":"application/pdf","url":"https://example.org/direct.pdf"}).to_string()).body(PDF).send().await.unwrap();
    assert_eq!(direct.status(), 201);
    let direct: Value = direct.json().await.unwrap();
    assert_eq!(direct["canRecognize"], false);
    assert_eq!(direct["metadataPending"], true);
    let recognized: Value = server
        .post("getRecognizedItem")
        .json(&json!({"sessionID":"standalone"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(recognized.is_null());
    let selected: Value = server
        .post("getSelectedCollection")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(selected["id"], "L1");
    let unsupported = server
        .post("updateSession")
        .json(&json!({"sessionID":"capture","target":"C1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), 501);
    let unsupported = server
        .post("saveSnapshot")
        .json(&json!({"pdf":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), 501);
    let update: Value = server
        .post("updateSession")
        .json(&json!({"sessionID":"capture","target":"L1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(update, json!({"sessionID":"capture"}));
    let items: Vec<_> = (0..128)
        .map(|index| json!({"id":index,"itemType":"document","title":format!("Bounded item {index}")}))
        .collect();
    let mut maximum = json!({"sessionID":"maximum","items":items});
    assert_eq!(
        save(&server, &maximum).await["items"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    maximum["sessionID"] = json!("over-limit");
    maximum["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":128,"itemType":"document","title":"One too many"}));
    assert_eq!(
        server
            .post("saveItems")
            .json(&maximum)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let oversized = "x".repeat(1024 * 1024 + 1);
    assert_eq!(
        server
            .post("saveItems")
            .body(oversized)
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let mut pdf = vec![b' '; 64 * 1024 * 1024];
    pdf[..PDF.len()].copy_from_slice(PDF);
    let response = server
        .post("saveStandaloneAttachment?sessionID=maximum-pdf")
        .header("Content-Type", "application/pdf")
        .header(
            "X-Metadata",
            json!({"title":"maximum.pdf","contentType":"application/pdf"}).to_string(),
        )
        .body(pdf)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    assert!(
        server
            .oversized_pdf_status()
            .await
            .starts_with("HTTP/1.1 413")
    );
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the current independently built literature package"]
async fn actual_package_http_failures_preserve_original_atomicity_parent_state_and_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    server.shared.store.connection().unwrap().execute_batch("create trigger synthetic_capture_receipt_failure before update of receipt_json on browser_capture_sessions begin select raise(abort,'synthetic capture failure'); end;").unwrap();
    let failed = server
        .post("saveItems")
        .json(&payload())
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), 500);
    let failed: Value = failed.json().await.unwrap();
    assert_eq!(failed["error"], "storage_error");
    assert!(failed["requestId"].is_string());
    assert!(failed["packageKey"].is_string());
    assert_eq!(
        server
            .shared
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from bibliographic_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    server
        .shared
        .store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_capture_receipt_failure")
        .unwrap();
    let receipt = save(&server, &payload()).await;
    let id = receipt["items"][0]["itemID"].as_str().unwrap();
    server
        .shared
        .store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set archived_at='now' where id=?1",
            [id],
        )
        .unwrap();
    let missing = server
        .post("saveAttachment?sessionID=capture")
        .header("Content-Type", "application/pdf")
        .header("X-Metadata", metadata("pdf", "parent").to_string())
        .body(PDF)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    server
        .shared
        .store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set archived_at=null where id=?1",
            [id],
        )
        .unwrap();
    server.shared.store.connection().unwrap().execute_batch("create trigger synthetic_capture_pdf_failure before insert on browser_capture_attachments begin select raise(abort,'synthetic capture failure'); end;").unwrap();
    let failed = server
        .post("saveAttachment?sessionID=capture")
        .header("Content-Type", "application/pdf")
        .header("X-Metadata", metadata("pdf", "parent").to_string())
        .body(PDF)
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), 500);
    assert_eq!(
        server
            .shared
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from attachments", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    server
        .shared
        .store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_capture_pdf_failure")
        .unwrap();
    attach(&server, "capture", metadata("pdf", "parent")).await;
    let conflict=server.post("saveItems").json(&json!({"sessionID":"capture","items":[{"id":"parent","itemType":"document","title":"Changed"}]})).send().await.unwrap();
    assert_eq!(conflict.status(), 409);
    let unauthorized = server
        .client
        .post(format!("http://{}/connector/saveItems", server.shared.host))
        .header("X-Zotero-Connector-API-Version", "3")
        .header("X-Research-Assistant-Connector-API-Version", "2")
        .header(INSTANCE_HEADER, &server.shared.instance_id)
        .header(TOKEN_HEADER, "0".repeat(64))
        .json(&payload())
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the current independently built literature package"]
async fn actual_package_http_unknown_commit_fences_storage_and_reopen_recovers_original_session() {
    let directory = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    server
        .shared
        .store
        .connection()
        .unwrap()
        .commit_hook(Some(|| true));
    let response = server
        .post("saveItems")
        .json(&payload())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 500);
    let response: Value = response.json().await.unwrap();
    assert_eq!(response["error"], "storage_error");
    assert_eq!(response["saveOutcome"]["status"], "unknown");
    assert!(response["requestId"].is_string());
    assert!(server.shared.store.connection().is_err());
    server.shutdown_fenced().await;
    let reopened = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    assert_eq!(
        reopened
            .shared
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from browser_capture_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let saved = save(&reopened, &payload()).await;
    assert_eq!(save(&reopened, &payload()).await, saved);
    reopened.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the current independently built literature package"]
async fn actual_package_http_late_identifier_error_retains_sql_precedence_and_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_binding(directory.path(), Some(selected())).await;
    let mut raw = payload();
    raw["items"][0]["ISBN"] = json!(17);
    server.shared.store.connection().unwrap().execute_batch("create trigger synthetic_capture_insert_failure before insert on bibliographic_items begin select raise(abort,'private fixture cause'); end;").unwrap();
    let first = server.post("saveItems").json(&raw).send().await.unwrap();
    assert_eq!(first.status(), 500);
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["error"], "storage_error");
    assert!(!first.to_string().contains("private fixture cause"));
    server
        .shared
        .store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_capture_insert_failure")
        .unwrap();
    let invalid = server.post("saveItems").json(&raw).send().await.unwrap();
    assert_eq!(invalid.status(), 400);
    assert_eq!(
        server
            .shared
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from browser_capture_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    save(&server, &payload()).await;
    server.shutdown().await;
}
