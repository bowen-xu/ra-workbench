//! Real Native decisions inside the original queue preview and compact owners.
use super::*;
use crate::agent::{
    writing_queue_preview::{self, QueuePreviewRequest},
    writing_queued_compaction::{self, Prepare},
};
use research_agent_core::input_queue::{QueueEdit, QueuedInput};
use serde_json::{Value, json};

#[derive(Clone)]
enum Wire {
    Preview(QueuePreviewRequest),
    Compact(Prepare),
}
impl Wire {
    fn new(input: &WritingSubmissionRequest, id: Uuid, compact: bool) -> Self {
        let submission = input.wire_request();
        if compact {
            Self::Compact(Prepare {
                input_id: id,
                expected_queue_revision: 1,
                previous_request_id: None,
                submission,
                expected_revision: 0,
            })
        } else {
            Self::Preview(QueuePreviewRequest {
                input_id: id,
                expected_revision: 1,
                submission,
            })
        }
    }
    async fn run(&self, f: &Fixture, service: &WritingSubmissionService) -> Result<Value, Value> {
        match self {
            Self::Preview(request) => {
                writing_queue_preview::prepare(service, &f.models, request.clone())
                    .await
                    .map(|value| serde_json::to_value(value).unwrap())
                    .map_err(|value| serde_json::to_value(value).unwrap())
            }
            Self::Compact(request) => {
                writing_queued_compaction::prepare(service, &f.models, request.clone())
                    .await
                    .map(|value| serde_json::to_value(value).unwrap())
                    .map_err(|value| serde_json::to_value(value).unwrap())
            }
        }
    }
}
async fn enqueue(
    f: &Fixture,
    service: &WritingSubmissionService,
    compact: bool,
) -> (Uuid, WritingSubmissionRequest) {
    let id = Uuid::new_v4();
    let mut input = request(f, Uuid::new_v4());
    if compact {
        input.text = "/compact".into();
    }
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    (id, input)
}
fn assert_unclaimed(f: &Fixture, request: Uuid) {
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    for (table, field) in [
        ("agent_submissions", "request_id"),
        ("agent_input_queue_receipts", "request_id"),
        ("data_sql_operation_receipts", "operation_id"),
        ("agent_compaction_preparations", "request_id"),
    ] {
        let count: u64 = db
            .query_row(
                &format!("select count(*) from {table} where {field}=?1"),
                [request.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_queue_preparations_freeze_original_claim_and_replay_before_mutable_sources()
{
    for compact in [false, true] {
        let f = fixture(true);
        let binding = binding(packages());
        let service = WritingSubmissionService::from_app_store(&f.store)
            .with_runtime_context(binding.clone());
        let (id, input) = enqueue(&f, &service, compact).await;
        let wire = Wire::new(&input, id, compact);
        let saved = wire.run(&f, &service).await.unwrap();
        assert_eq!(
            saved["preview"]["input"]["context"][0]["text"],
            "Frozen manuscript"
        );
        assert_eq!(count(&f), 1);
        let queue = service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap();
        assert_eq!(queue.revision(), 2);
        assert_eq!(queue.preparing(), Some(id));
        binding.service.shutdown().await.unwrap();
        std::fs::remove_file(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex"),
        )
        .unwrap();
        f.store
            .connection()
            .unwrap()
            .execute("update llm_model_routes set model_id='changed-default'", [])
            .unwrap();
        crate::ai::JsonCredentialStore::from_path(f.root.path().join("credentials.json"))
            .delete("deepseek")
            .unwrap();
        assert_eq!(wire.run(&f, &service).await.unwrap(), saved);
        assert_eq!(count(&f), 1);
        let purpose: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_compaction_preparations where request_id=?1",
                [input.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(purpose, u64::from(compact));
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_queue_rejection_keeps_original_class_and_adds_precise_native_provenance() {
    for compact in [false, true] {
        let f = fixture(true);
        let binding = binding(packages());
        let service = WritingSubmissionService::from_app_store(&f.store)
            .with_runtime_context(binding.clone());
        let (id, mut input) = enqueue(&f, &service, compact).await;
        input.context.files[0].expected_disk_sha256 = "f".repeat(64);
        let error = Wire::new(&input, id, compact)
            .run(&f, &service)
            .await
            .unwrap_err();
        if compact {
            assert_eq!(error["code"], "queue");
            assert_eq!(error["reason"]["code"], "unavailable");
        } else {
            assert_eq!(error["code"], "storage");
            assert_eq!(error["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
        }
        let detail = &error["writingSource"];
        assert_eq!(detail["conversationId"], f.conversation.to_string());
        assert_eq!(detail["manuscriptId"], f.manuscript.to_string());
        assert_eq!(detail["operationId"], input.request_id.to_string());
        assert_eq!(detail["inputId"], id.to_string());
        assert_eq!(detail["source"]["kind"], "native");
        assert_eq!(detail["source"]["packageKey"], binding.package_key);
        assert_eq!(detail["source"]["rejection"], "conflict");
        for key in ["requestId", "instanceId"] {
            assert!(Uuid::parse_str(detail["source"][key].as_str().unwrap()).is_ok());
        }
        for private in [
            "Frozen manuscript",
            "synthetic-fixture",
            f.root.path().to_str().unwrap(),
        ] {
            assert!(!error.to_string().contains(private));
        }
        assert_unclaimed(&f, input.request_id);
        assert!(
            service
                .read_input_queue(f.manuscript, f.conversation)
                .await
                .unwrap()
                .preparing()
                .is_none()
        );
        binding.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_queued_native_wait_releases_sql_and_rechecks_original_queue_revision() {
    for compact in [false, true] {
        let f = Arc::new(fixture(true));
        let (binding, pause) = super::pause::paused_binding(f.root.path());
        let service = WritingSubmissionService::from_app_store(&f.store)
            .with_runtime_context(binding.clone());
        let (id, input) = enqueue(&f, &service, compact).await;
        let operation = input.request_id;
        let wire = Wire::new(&input, id, compact);
        let job = tokio::spawn({
            let f = Arc::clone(&f);
            let service = service.clone();
            async move { wire.run(&f, &service).await }
        });
        pause.entered().await;
        tokio::time::timeout(Duration::from_secs(3), async {
            assert_unclaimed(&f, operation);
            service
                .edit_input_queue(
                    f.manuscript,
                    f.conversation,
                    Uuid::new_v4(),
                    1,
                    QueueEdit::Add(QueuedInput {
                        id: Uuid::new_v4(),
                        text: "Later".into(),
                        user_input: vec![],
                        origin: None,
                    }),
                )
                .await
                .unwrap();
        })
        .await
        .expect("Native wait must leave the original data owner available");
        pause.release();
        let error = job.await.unwrap().unwrap_err();
        if compact {
            assert_eq!(error, json!({"code":"queue","reason":{"code":"conflict"}}));
        } else {
            assert_eq!(error["reason"]["code"], "DATA_SQL_DOMAIN");
        }
        assert_unclaimed(&f, operation);
        let queue = service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap();
        assert_eq!(queue.revision(), 2);
        assert!(queue.preparing().is_none());
        binding.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_queued_waiter_cancellation_keeps_owner_and_exactly_one_original_claim() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Lease(Arc<AtomicBool>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    for compact in [false, true] {
        let f = Arc::new(fixture(true));
        let (binding, pause) = super::pause::paused_binding(f.root.path());
        let mut service = WritingSubmissionService::from_app_store(&f.store)
            .with_runtime_context(binding.clone());
        let (id, input) = enqueue(&f, &service, compact).await;
        let wire = Wire::new(&input, id, compact);
        let retry = wire.clone();
        let released = Arc::new(AtomicBool::new(false));
        service.worker_retention =
            crate::agent::repository::WorkerRetention::new(Lease(Arc::clone(&released)));
        let job = tokio::spawn({
            let f = Arc::clone(&f);
            async move { wire.run(&f, &service).await }
        });
        pause.entered().await;
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        assert!(!released.load(Ordering::SeqCst));
        assert_unclaimed(&f, input.request_id);
        pause.release();
        tokio::time::timeout(Duration::from_secs(20), async {
            while !released.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(count(&f), 1);
        let service = WritingSubmissionService::from_app_store(&f.store)
            .with_runtime_context(binding.clone());
        retry.run(&f, &service).await.unwrap();
        assert_eq!(count(&f), 1);
        let queue = service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap();
        assert_eq!(queue.revision(), 2);
        assert_eq!(queue.preparing(), Some(id));
        binding.service.shutdown().await.unwrap();
    }
}

#[test]
fn queued_compact_native_transport_failure_keeps_error_and_both_request_identities() {
    use crate::{
        agent::repository::input_queue::queued_compaction::PrepareIntent,
        runtime_plugins::native::{NativeHostError, NativeProfile},
    };
    let command = PrepareIntent {
        conversation_id: Uuid::new_v4(),
        manuscript_id: Uuid::new_v4(),
        input_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        expected_queue_revision: 1,
        expected_revision: 0,
        previous_request_id: None,
        request_json: json!({}),
    };
    let native_id = Uuid::new_v4();
    let mut native = NativeHostError::for_profile(
        NativeProfile::WritingProjects,
        "plugin_host_exited",
        "写作宿主已退出。",
    );
    native.request_id = Some(native_id.to_string());
    native.exit_code = Some(43);
    let value = serde_json::to_value(writing_queued_compaction::Error::source(
        native.into(),
        &command,
    ))
    .unwrap();
    assert_eq!(value["code"], "preparation");
    assert_eq!(value["reason"], "unavailable");
    let detail = &value["writingSource"];
    assert_eq!(detail["operationId"], command.request_id.to_string());
    assert_eq!(detail["source"]["kind"], "nativeFailure");
    assert_eq!(detail["source"]["error"]["code"], "plugin_host_exited");
    assert_eq!(
        detail["source"]["error"]["requestId"],
        native_id.to_string()
    );
    assert_eq!(detail["source"]["error"]["exitCode"], 43);
}
