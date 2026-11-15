use std::{fs, path::PathBuf, time::Duration};

use fs2::FileExt;
use rusqlite::Connection;
use serde_json::json;

use crate::{docling::repository, storage::migrate};

use super::{
    error::DoclingError,
    launcher::acquire_start_lock,
    manager::{DoclingServiceManager, build_convert_pdf_payload},
    model::{DoclingSettings, DoclingSettingsDraft},
    protocol::{
        DaemonStatus, LeaseGranted, LeaseHeartbeatAck, RegistryRecord, validate_registry,
        validate_status,
    },
    repository::ParsedDoclingResponse,
    should_start_on_app_start,
};

fn migrated_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open in-memory database");
    migrate(&mut connection).expect("apply migrations");
    connection
}

fn insert_docling_attachment_fixture(connection: &Connection) {
    connection
        .execute(
            "insert into bibliographic_items (id, item_type, title, created_at, updated_at)
             values ('paper-1', 'document', 'Paper', '2026-08-09T00:00:00.000Z', '2026-08-09T00:00:00.000Z')",
            [],
        )
        .expect("insert paper");
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values ('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'aa/file.pdf', 'application/pdf', 10, '2026-08-09T00:00:00.000Z')",
            [],
        )
        .expect("insert content object");
    connection
        .execute(
            "insert into attachments (id, item_id, content_sha256, display_name, created_at)
             values ('att-1', 'paper-1', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'file.pdf', '2026-08-09T00:00:00.000Z')",
            [],
        )
        .expect("insert attachment");
}

fn save_docling_parse_run(connection: &Connection, parse_status: &str, created_at: &str) -> String {
    let saved = repository::save_parse_response(
        connection,
        ParsedDoclingResponse {
            item_id: "paper-1".to_owned(),
            attachment_id: "att-1".to_owned(),
            content_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            service_base_url: "http://127.0.0.1:5001".to_owned(),
            docling_status: parse_status.to_owned(),
            parse_status: parse_status.to_owned(),
            json_content: if parse_status == "failure" {
                None
            } else {
                Some(json!({ "pages": [] }))
            },
            raw_response: json!({ "status": parse_status }),
            error_message: None,
            processing_time_seconds: Some(1.25),
        },
    )
    .expect("save parse response");
    connection
        .execute(
            "update docling_parse_runs set created_at = ?1, finished_at = ?1 where id = ?2",
            [created_at, saved.summary.id.as_str()],
        )
        .expect("update parse run time");
    saved.summary.id
}

#[test]
fn saves_docling_settings_with_fallback_range() {
    let connection = migrated_connection();
    let draft = DoclingSettingsDraft {
        enabled: true,
        model_manifest_url: "https://example.com/models/latest.json".to_owned(),
        default_port: 5001,
        fallback_start_port: 5010,
        fallback_end_port: 5020,
        idle_shutdown_ms: 600_000,
        request_timeout_ms: 300_000,
    };

    let settings = repository::save_settings(&connection, &draft).expect("save settings");

    assert!(settings.enabled);
    assert_eq!(
        settings.model_manifest_url,
        "https://example.com/models/latest.json"
    );
    assert_eq!(settings.default_port, 5001);
    assert_eq!(settings.fallback_end_port, 5020);
}

#[test]
fn rejects_invalid_fallback_range() {
    let connection = migrated_connection();
    let draft = DoclingSettingsDraft {
        enabled: true,
        model_manifest_url: "http://127.0.0.1:4174/models/docling/latest.json".to_owned(),
        default_port: 5001,
        fallback_start_port: 5020,
        fallback_end_port: 5010,
        idle_shutdown_ms: 600_000,
        request_timeout_ms: 300_000,
    };

    assert!(repository::save_settings(&connection, &draft).is_err());
}

#[test]
fn builds_docling_convert_source_payload_for_file_sources() {
    let payload = build_convert_pdf_payload(b"pdf bytes", "paper.pdf");

    assert!(payload.get("file_sources").is_none());
    assert_eq!(payload["sources"][0]["kind"], "file");
    assert_eq!(payload["sources"][0]["filename"], "paper.pdf");
    assert_eq!(payload["sources"][0]["base64_string"], "cGRmIGJ5dGVz");
    assert_eq!(payload["options"]["from_formats"], json!(["pdf"]));
    assert_eq!(payload["options"]["to_formats"], json!(["json"]));
}

#[test]
fn startup_policy_follows_enabled_setting() {
    let connection = migrated_connection();
    let mut settings = repository::get_settings(&connection).expect("load settings");

    assert!(should_start_on_app_start(&settings));
    assert_eq!(
        settings.model_manifest_url,
        "http://127.0.0.1:4174/models/docling/latest.json"
    );

    settings.enabled = false;
    assert!(!should_start_on_app_start(&settings));
}

fn registry_record() -> RegistryRecord {
    RegistryRecord {
        model_identity: None,
        precision: None,
        schema_version: 1,
        protocol_version: 2,
        service_instance_id: "service-1".to_owned(),
        state: "ready".to_owned(),
        http_base_url: "http://127.0.0.1:5001".to_owned(),
        websocket_url: "ws://127.0.0.1:5001/local-service/v1/lease".to_owned(),
        pid: 123,
        started_at: "2026-08-10T00:00:00Z".to_owned(),
        wrapper_version: "1".to_owned(),
        docling_serve_version: "1.30.0".to_owned(),
        heartbeat_interval_ms: 10_000,
        lease_timeout_ms: 30_000,
        idle_shutdown_ms: 1_200_000,
    }
}

#[test]
fn accepts_matching_loopback_daemon_registry_and_status() {
    let registry = registry_record();
    let mut status = DaemonStatus {
        service_revision: research_docling_runtime::SERVICE_REVISION,
        model_identity: None,
        precision: None,
        schema_version: 1,
        protocol_version: 2,
        service_instance_id: "service-1".to_owned(),
        state: "ready".to_owned(),
        active_client_count: Some(1),
        active_request_count: Some(0),
    };

    validate_registry(&registry).expect("validate registry");
    validate_status(&registry, &status).expect("validate status");
    status.service_revision = 0;
    assert!(matches!(
        validate_status(&registry, &status),
        Err(DoclingError::Incompatible(_))
    ));
}

#[test]
fn lease_messages_allow_missing_observability_counts() {
    let granted: LeaseGranted = serde_json::from_value(json!({
        "type": "lease",
        "leaseId": "lease-1",
        "serviceInstanceId": "service-1",
        "heartbeatIntervalMs": 10_000,
        "leaseTimeoutMs": 30_000
    }))
    .expect("decode lease without counts");
    let ack: LeaseHeartbeatAck = serde_json::from_value(json!({
        "type": "heartbeatAck",
        "leaseId": "lease-1"
    }))
    .expect("decode heartbeat ack without counts");

    assert_eq!(granted.active_client_count, 1);
    assert_eq!(granted.active_request_count, None);
    assert_eq!(ack.active_client_count, None);
    assert_eq!(ack.active_request_count, None);
}

#[test]
fn rejects_non_loopback_or_incompatible_daemon_registry() {
    let mut registry = registry_record();
    registry.http_base_url = "http://0.0.0.0:5001".to_owned();
    assert!(validate_registry(&registry).is_err());

    let mut registry = registry_record();
    registry.protocol_version = 999;
    assert!(matches!(
        validate_registry(&registry),
        Err(DoclingError::Incompatible(_))
    ));
}

fn temporary_service_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "research-assistant-docling-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path).expect("create service directory");
    path
}

#[tokio::test]
async fn parsing_never_starts_a_disconnected_service_and_startup_is_attempted_once() {
    let directory = tempfile::tempdir().unwrap();
    let manager = DoclingServiceManager::from_paths(
        directory.path().into(),
        directory.path().join("not-executable"),
    );
    let settings = docling_settings();
    assert!(matches!(
        manager.require_connected(&settings).await,
        Err(DoclingError::ManualStartRequired)
    ));
    let clone = manager.clone();
    let (first, second) = tokio::join!(
        manager.start_on_app_start(&settings),
        clone.start_on_app_start(&settings)
    );
    let errors = [first.unwrap_err(), second.unwrap_err()];
    assert_eq!(
        errors
            .iter()
            .filter(|e| matches!(e, DoclingError::Models(_)))
            .count(),
        1
    );
    assert_eq!(
        errors
            .iter()
            .filter(|e| matches!(e, DoclingError::ManualStartRequired))
            .count(),
        1
    );
    assert!(matches!(
        manager.require_connected(&settings).await,
        Err(DoclingError::ManualStartRequired)
    ));
    // Explicit connection still validates models and can retry a failed startup.
    assert!(matches!(
        manager.start_service(&settings).await,
        Err(DoclingError::Models(_))
    ));
    assert!(matches!(
        manager.start_on_app_start(&settings).await,
        Err(DoclingError::ManualStartRequired)
    ));
}

fn docling_settings() -> DoclingSettings {
    DoclingSettings {
        enabled: true,
        model_manifest_url: "http://127.0.0.1:4174/models/docling/latest.json".to_owned(),
        default_port: 5001,
        fallback_start_port: 5010,
        fallback_end_port: 5099,
        idle_shutdown_ms: 60_000,
        request_timeout_ms: 300_000,
        updated_at: "2026-08-10T00:00:00Z".to_owned(),
    }
}

#[tokio::test]
async fn live_unresponsive_registry_is_not_replaced() {
    let directory = temporary_service_directory();
    let mut registry = registry_record();
    registry.pid = std::process::id();
    registry.http_base_url = "http://127.0.0.1:9".to_owned();
    registry.websocket_url = "ws://127.0.0.1:9/local-service/v1/lease".to_owned();
    fs::write(
        directory.join("docling-service.json"),
        serde_json::to_vec(&registry).expect("serialize registry"),
    )
    .expect("write registry");
    fs::write(directory.join("docling-service.token"), "secret").expect("write token");
    let manager = DoclingServiceManager::from_paths(directory.clone(), directory.join("daemon.py"));

    // Automatic reconnection may not adopt an unrelated registry instance,
    // even if its PID is alive. Explicit connection still reports its cause.
    assert!(matches!(
        manager
            .discover_and_connect(&docling_settings(), false)
            .await,
        Err(DoclingError::ManualStartRequired)
    ));
    let result = manager
        .discover_and_connect(&docling_settings(), true)
        .await;

    assert!(matches!(result, Err(DoclingError::Unresponsive(_))));
    assert!(directory.join("docling-service.json").exists());
    fs::remove_dir_all(directory).expect("remove service directory");
}

#[tokio::test]
async fn startup_lock_serializes_concurrent_clients() {
    let directory = temporary_service_directory();
    let lock_path = directory.join("start.lock");
    let first = acquire_start_lock(lock_path.clone())
        .await
        .expect("first lock");
    let mut second = tokio::spawn(async move { acquire_start_lock(lock_path).await });

    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut second)
            .await
            .is_err()
    );
    FileExt::unlock(&first).expect("unlock first client");
    let second_lock = tokio::time::timeout(Duration::from_secs(1), second)
        .await
        .expect("second client obtains lock")
        .expect("join second lock")
        .expect("acquire second lock");
    FileExt::unlock(&second_lock).expect("unlock second client");
    fs::remove_dir_all(directory).expect("remove service directory");
}

#[tokio::test]
#[ignore = "requires built native daemon, verified FP32 models and loopback sockets"]
async fn concurrent_rust_clients_share_daemon_and_allow_idle_exit() {
    let directory = temporary_service_directory();
    let source = PathBuf::from(
        std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_MODEL_DIRECTORY")
            .expect("set model directory"),
    );
    let manifest: research_docling_runtime::ModelManifest =
        serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    let name = manifest.directory_name().unwrap();
    let daemon_script = std::env::var_os("RESEARCH_ASSISTANT_DOCLING_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("docling-daemon/target/release/research-docling-daemon")
        });
    if let Ok(url) = std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_MODEL_SOURCE") {
        let resources =
            super::models::ModelResources::native(directory.join("models"), daemon_script.clone());
        let installed = resources
            .install(&url)
            .await
            .expect("download and validate real models");
        assert_eq!(
            installed.installed_identity.as_deref(),
            Some(manifest.identity().unwrap().as_str())
        );
    } else {
        let target = directory.join("models").join(&name);
        fs::create_dir_all(&target).unwrap();
        for file in &manifest.files {
            let destination = target.join(&file.path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::hard_link(source.join(&file.path), destination).unwrap();
        }
        let bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(target.join("manifest.json"), &bytes).unwrap();
        fs::write(directory.join("models/active.json"), &bytes).unwrap();
    }
    let first = DoclingServiceManager::from_paths(directory.clone(), daemon_script.clone());
    let second = DoclingServiceManager::from_paths(directory.clone(), daemon_script);
    let mut settings = docling_settings();
    settings.default_port = 0;
    settings.fallback_start_port = 1;
    settings.fallback_end_port = 1;
    settings.idle_shutdown_ms = 250;

    let (first_service, second_service) = tokio::join!(
        first.start_service(&settings),
        second.start_service(&settings)
    );
    let first_service = first_service.expect("connect first client");
    let second_service = second_service.expect("connect second client");
    assert_eq!(first_service.instance_id, second_service.instance_id);
    assert!(first.status().active_client_count.is_some());
    assert!(second.status().active_client_count.is_some());
    assert_eq!(first.refreshed_status().await.active_client_count, Some(2));
    let ping = first.ping().await.expect("ping shared daemon");
    assert_eq!(ping.status.active_client_count, Some(2));

    first.disconnect().await.expect("disconnect first client");
    assert_eq!(second.status().state, "connected");
    second.disconnect().await.expect("disconnect second client");

    let registry = directory
        .join("instances")
        .join(manifest.instance_directory_name().unwrap())
        .join("docling-service.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while registry.exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !registry.exists(),
        "daemon should remove its registry on idle exit"
    );
    fs::remove_dir_all(directory).expect("remove service directory");
}

#[test]
fn stores_raw_docling_response_and_json_content() {
    let connection = migrated_connection();
    insert_docling_attachment_fixture(&connection);

    let saved = repository::save_parse_response(
        &connection,
        ParsedDoclingResponse {
            item_id: "paper-1".to_owned(),
            attachment_id: "att-1".to_owned(),
            content_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            service_base_url: "http://127.0.0.1:5001".to_owned(),
            docling_status: "success".to_owned(),
            parse_status: "success".to_owned(),
            json_content: Some(json!({ "body": { "children": [] } })),
            raw_response: json!({
                "status": "success",
                "document": { "json_content": { "body": { "children": [] } } }
            }),
            error_message: None,
            processing_time_seconds: Some(1.25),
        },
    )
    .expect("save parse response");

    assert_eq!(saved.summary.attachment_id, "att-1");
    assert_eq!(saved.summary.parse_status, "success");
    assert_eq!(
        saved.json_content.expect("json content")["body"]["children"],
        json!([])
    );
    assert_eq!(
        repository::list_parse_runs(&connection, "paper-1")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn selects_latest_successful_docling_parse_run_for_overlay() {
    let connection = migrated_connection();
    insert_docling_attachment_fixture(&connection);
    let older_success = save_docling_parse_run(&connection, "success", "2026-08-09T00:00:00.000Z");
    let latest_partial =
        save_docling_parse_run(&connection, "partial_success", "2026-08-09T00:10:00.000Z");
    let _ignored_failure =
        save_docling_parse_run(&connection, "failure", "2026-08-09T00:20:00.000Z");

    let selected = super::overlay_repository::read_page(&connection, "att-1", 0, None)
        .expect("select latest successful parse");

    assert_ne!(selected.parse_run_id, older_success);
    assert_eq!(selected.parse_run_id, latest_partial);
}

#[test]
fn reports_missing_docling_overlay_parse_run_without_successful_result() {
    let connection = migrated_connection();
    insert_docling_attachment_fixture(&connection);
    let _failure = save_docling_parse_run(&connection, "failure", "2026-08-09T00:20:00.000Z");

    let result = super::overlay_repository::read_page(&connection, "att-1", 0, None);

    assert!(matches!(result, Err(DoclingError::ParseRunNotFound)));
}

#[test]
fn native_model_settings_upgrade_preserves_old_config_parse_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.sqlite");
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 182);
        db.execute("update docling_settings set conda_environment='legacy-research',request_timeout_ms=120000 where id=1",[]).unwrap();
        insert_docling_attachment_fixture(&db);
        save_docling_parse_run(&db, "success", "2026-10-01T00:00:00Z");
        migrate(&mut db).unwrap();
        assert_eq!(
            repository::get_settings(&db).unwrap().model_manifest_url,
            research_docling_runtime::DEFAULT_MANIFEST_URL
        );
        assert_eq!(
            repository::get_settings(&db).unwrap().request_timeout_ms,
            120000
        );
        assert_eq!(
            repository::list_parse_runs(&db, "paper-1").unwrap().len(),
            1
        );
        let previous: String = db
            .query_row(
                "select conda_environment from docling_settings where id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(previous, "legacy-research");
    }
    let mut reopened = Connection::open(path).unwrap();
    migrate(&mut reopened).unwrap();
    assert_eq!(
        repository::list_parse_runs(&reopened, "paper-1")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn native_service_must_match_selected_fp32_model_identity() {
    let mut record = registry_record();
    record.model_identity = Some("expected-model".into());
    record.precision = Some("fp32".into());
    record.docling_serve_version = research_docling_runtime::ENGINE_VERSION.into();
    let mut status: DaemonStatus = serde_json::from_value(json!({
        "schemaVersion":1,"protocolVersion":2,"serviceInstanceId":record.service_instance_id,
        "state":"ready","modelIdentity":"expected-model","precision":"fp32"
    }))
    .unwrap();
    assert!(super::protocol::validate_model_identity(&record, &status, "expected-model").is_ok());
    status.model_identity = Some("different-model".into());
    assert!(super::protocol::validate_model_identity(&record, &status, "expected-model").is_err());
    status.model_identity = record.model_identity.clone();
    status.precision = Some("int8".into());
    assert!(super::protocol::validate_model_identity(&record, &status, "expected-model").is_err());
}
