//! Real independent context decisions, through the existing Core prepare path.
use super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn packages() -> PathBuf {
    let directory = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
            .expect("Set the actual independent package directory"),
    )
    .unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    assert!(directory.starts_with(repository.join(".tmp")));
    directory
}
fn binding(directory: PathBuf) -> Binding {
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory)));
    let discovered = catalog.discover().unwrap();
    assert_eq!(discovered.packages.len(), 1);
    Binding {
        service: Arc::new(NativePreviewService::new(Arc::clone(&catalog))),
        catalog,
        package_key: discovered.packages[0].package_key.clone(),
    }
}

#[test]
fn native_prepare_failure_retains_original_and_native_request_identities() {
    use crate::{
        agent::writing_commands::PrepareRequest,
        runtime_plugins::native::{NativeHostError, NativeProfile},
    };
    let c = Uuid::new_v4();
    let m = Uuid::new_v4();
    let r = Uuid::new_v4();
    let stage = Uuid::new_v4();
    let request: PrepareRequest = serde_json::from_value(serde_json::json!({"conversationId":c,"requestId":r,"text":"Explain","context":{"manuscriptId":m,"files":[]}})).unwrap();
    let mut native = NativeHostError::for_profile(
        NativeProfile::WritingProjects,
        "plugin_host_exited",
        "写作宿主已退出。",
    );
    native.request_id = Some(stage.to_string());
    native.operation_id = Some(r.to_string());
    native.exit_code = Some(43);
    let value =
        serde_json::to_value(request.preparation_failure(WritingSubmissionError::from(native)))
            .unwrap();
    assert_eq!(value["code"], "plugin_host_exited");
    assert_eq!(value["conversationId"], c.to_string());
    assert_eq!(value["manuscriptId"], m.to_string());
    assert_eq!(value["operationId"], r.to_string());
    assert_eq!(value["requestId"], stage.to_string());
    assert_eq!(value["exitCode"], 43);
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_prepares_live_and_current_file_context_and_replays_original_receipt() {
    use crate::agent::{
        WritingContextBuffer, WritingContextSelection, context_session::sources::Resource,
    };
    let f = fixture(true);
    let binding = binding(packages());
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_runtime_context(binding.clone());
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].buffer = Some(WritingContextBuffer {
        revision: Uuid::nil(),
        text: "甲🧬乙".into(),
    });
    input.context.files[0].selection = Some(WritingContextSelection {
        start_utf16: 1,
        end_utf16: 3,
    });
    input.resources.push(Resource::ManuscriptFile {
        resource_id: f.manuscript,
        path: "main.tex".into(),
    });
    let original = service.prepare(&f.models, input.clone()).await.unwrap();
    assert_eq!(original.record.input.context[0].text, "🧬");
    assert_eq!(
        original.record.input.context[0].revision,
        format!("sha256:{:x}", Sha256::digest("甲🧬乙"))
    );
    assert_eq!(count(&f), 1);
    binding.service.shutdown().await.unwrap();
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    let replay = service.prepare(&f.models, input).await.unwrap();
    assert_eq!(replay.record, original.record);
    assert_eq!(count(&f), 1);
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_context_rejection_preserves_sql_class_native_identity_and_zero_write() {
    let f = fixture(true);
    let binding = binding(packages());
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_runtime_context(binding.clone());
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].expected_disk_sha256 = "f".repeat(64);
    let command = input.wire_request();
    let error = service.prepare(&f.models, input).await.err().unwrap();
    let json = serde_json::to_value(command.preparation_failure(error)).unwrap();
    assert_eq!(json["code"], "storage");
    assert_eq!(json["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert_eq!(
        json["reason"]["reason"]["safeCause"],
        "writing_context_changed"
    );
    assert_eq!(json["reason"]["source"]["kind"], "native");
    assert_eq!(json["reason"]["source"]["packageKey"], binding.package_key);
    assert!(
        json["reason"]["source"]["instanceId"]
            .as_str()
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
    );
    assert!(
        json["reason"]["source"]["requestId"]
            .as_str()
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
    );
    assert!(!json.to_string().contains("synthetic-fixture"));
    assert_eq!(count(&f), 0);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from data_sql_operation_receipts",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    binding.service.shutdown().await.unwrap();
}

#[path = "runtime_context/pause.rs"]
mod pause;

#[path = "runtime_context/queued.rs"]
mod queued;

#[path = "runtime_context/trusted.rs"]
mod trusted;

#[path = "runtime_context/host_resources.rs"]
mod host_resources;
