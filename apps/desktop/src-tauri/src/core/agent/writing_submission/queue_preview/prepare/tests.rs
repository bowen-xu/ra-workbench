use super::*;
use crate::agent::writing_submission::tests::{managed_fixture, queue_preview_intent, request};
use research_agent_core::input_queue::{QueueEdit, QueuedInput};

#[test]
fn model_selection_failures_have_only_safe_prewrite_classifications_without_sql_proof() {
    let operation = Uuid::new_v4();
    for (error, safe_cause) in [
        (
            AgentModelPreparationError::Settings,
            "model_settings_unavailable",
        ),
        (
            AgentModelPreparationError::Disabled,
            "model_provider_disabled",
        ),
        (
            AgentModelPreparationError::Credentials,
            "model_credentials_unavailable",
        ),
        (
            AgentModelPreparationError::ChatGptTransportUnavailable,
            "chatgpt_transport_unavailable",
        ),
        (
            AgentModelPreparationError::UnsupportedProtocol,
            "model_protocol_unsupported",
        ),
        (
            AgentModelPreparationError::Worker,
            "model_preparation_worker_unavailable",
        ),
        (
            AgentModelPreparationError::Transport(
                research_agent_core::model::ModelError::Unavailable,
            ),
            "model_transport_unavailable",
        ),
    ] {
        let value = serde_json::to_value(model_selection_failure(operation, error)).unwrap();
        assert_eq!(value["code"], "DATA_SQL_UNAVAILABLE");
        assert_eq!(value["reason"]["operationId"], operation.to_string());
        assert_eq!(value["reason"]["phase"], "queue-prepare-model-selection");
        assert_eq!(value["reason"]["safeCause"], safe_cause);
        assert_eq!(value["reason"].as_object().unwrap().len(), 7);
        for key in ["epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode"] {
            assert!(value["reason"][key].is_null());
        }
        assert!(value.get("proof").is_none());
    }
}

#[tokio::test]
async fn missing_model_credentials_leave_original_queue_and_no_prepare_receipt() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let input_id = Uuid::new_v4();
    let input = request(&f, Uuid::new_v4());
    let intent = queue_preview_intent(&input, input_id, 1);
    let queue = service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    crate::ai::JsonCredentialStore::from_path(f.root.path().join("credentials.json"))
        .delete("deepseek")
        .unwrap();
    let error = service
        .prepare_queue_preview(&f.models, intent.clone(), input)
        .await
        .unwrap_err();
    let WritingQueuePreviewError::Storage(SqlImmediateFailure::Unavailable(detail)) = error else {
        panic!("model credential failure must preserve its original operation")
    };
    assert_eq!(detail.operation_id, intent.request_id);
    assert_eq!(detail.phase, "queue-prepare-model-selection");
    assert_eq!(detail.safe_cause, "model_credentials_unavailable");
    assert!(detail.epoch.is_none());
    assert!(detail.sql_fence.is_none());
    assert!(matches!(
        service
            .read_queue_preview_prepare_status(intent.clone())
            .await
            .unwrap(),
        QueuePreviewPrepareStatus::NotFound
    ));
    assert_eq!(
        service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap(),
        queue
    );
    let db = f.store.connection().unwrap();
    let counts:(u64,u64)=db.query_row(
        "select (select count(*) from agent_submissions where request_id=?1), (select count(*) from data_sql_operation_receipts where operation_id=?1)",
        [intent.request_id.to_string()], |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn queued_model_selection_error_keeps_full_wire_identity_and_status_not_found_without_secrets()
 {
    use crate::agent::writing_queue_preview::{self, QueuePreviewRequest};
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let input_id = Uuid::new_v4();
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(AgentModelSelection {
        provider_id: "private-unregistered-provider".into(),
        model_id: "private-unregistered-model".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    let intent = queue_preview_intent(&input, input_id, 1);
    let wire: QueuePreviewRequest = serde_json::from_value(intent.request_json).unwrap();
    let error = writing_queue_preview::prepare(&service, &f.models, wire.clone())
        .await
        .err()
        .unwrap();
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "storage");
    let reason = &value["reason"];
    assert_eq!(reason["code"], "DATA_SQL_UNAVAILABLE");
    assert_eq!(reason["conversationId"], f.conversation.to_string());
    assert_eq!(reason["manuscriptId"], f.manuscript.to_string());
    assert_eq!(reason["inputId"], input_id.to_string());
    assert_eq!(reason["previewRequestId"], intent.request_id.to_string());
    assert_eq!(reason["operationId"], intent.request_id.to_string());
    assert_eq!(reason["reason"]["phase"], "queue-prepare-model-selection");
    assert_eq!(reason["reason"]["safeCause"], "model_settings_unavailable");
    for key in ["epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode"] {
        assert!(reason["reason"][key].is_null());
    }
    let json = value.to_string();
    for private in [
        "private-unregistered-provider",
        "private-unregistered-model",
        "synthetic-fixture",
        f.root.path().to_str().unwrap(),
    ] {
        assert!(!json.contains(private));
    }
    assert!(value.get("proof").is_none());
    let status = serde_json::to_value(
        writing_queue_preview::prepare_status(&service, wire.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status["intent"], serde_json::to_value(&wire).unwrap());
    assert_eq!(status["status"], serde_json::json!({"type":"notFound"}));
    let ordinary = crate::agent::writing_commands::prepare(&service, &f.models, wire.submission)
        .await
        .err()
        .unwrap();
    assert_eq!(serde_json::to_value(ordinary).unwrap(), "modelUnavailable");
}

#[tokio::test]
async fn exact_prepare_restores_original_result_before_changed_sources_routes_and_credentials() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let input = request(&f, request_id);
    let intent = queue_preview_intent(&input, input_id, 1);
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    let prepared = service
        .prepare_queue_preview(&f.models, intent.clone(), input)
        .await
        .unwrap();
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
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            2,
            QueueEdit::Add(QueuedInput {
                id: Uuid::new_v4(),
                text: "Later".into(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    let replay = service
        .prepare_queue_preview(&f.models, intent.clone(), request(&f, request_id))
        .await
        .unwrap();
    assert_eq!(replay.record, prepared.record);
    assert_eq!(replay.queue, prepared.queue);
    assert_eq!(replay.queue.revision(), 2);
    assert_eq!(
        service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap()
            .revision(),
        3
    );
    let QueuePreviewPrepareStatus::Committed { state, receipt } = service
        .read_queue_preview_prepare_status(intent)
        .await
        .unwrap()
    else {
        panic!("original proof")
    };
    assert_eq!(state.record, prepared.record);
    assert_eq!(state.queue, prepared.queue);
    assert!(receipt.replayed);
    assert!(
        service
            .restore(
                &f.models,
                WritingSubmissionReceipt {
                    conversation_id: f.conversation,
                    manuscript_id: f.manuscript,
                    request_id,
                }
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn queued_prepare_freezes_selected_readonly_scope_and_rejects_changed_resources_for_same_r() {
    use crate::agent::context_session::{
        permissions,
        sources::{Access, Part, Resource},
    };
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let request_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let mut input = request(&f, request_id);
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: None,
        group_access: None,
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: "main.tex".into(),
            },
            label: "main.tex".into(),
            selected: true,
            access: Access::ReadOnly,
        }],
    }];
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: input.text.clone(),
                user_input: vec![],
                origin: None,
            }),
        )
        .await
        .unwrap();
    let original = service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, input_id, 1), input)
        .await
        .unwrap();
    assert_eq!(
        permissions::writing_paths(&original.record),
        Some(std::collections::HashSet::new())
    );
    let changed = request(&f, request_id);
    let error = service
        .prepare_queue_preview(
            &f.models,
            queue_preview_intent(&changed, input_id, 1),
            changed,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WritingQueuePreviewError::Storage(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(
        service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap(),
        original.queue
    );
}
