use super::*;
use crate::data_interface::SqlImmediateDiagnostic;
use serde_json::json;

fn request() -> CancelRequest {
    CancelRequest {
        conversation_id: Uuid::new_v4(),
        manuscript_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        expected_revision: 2,
        input_id: Uuid::new_v4(),
        preview_request_id: Uuid::new_v4(),
    }
}

#[test]
fn status_echoes_the_full_canonical_intent_and_reuses_the_specific_proof_family() {
    let request = request();
    let epoch = Uuid::new_v4();
    let intent = serde_json::to_value(&request).unwrap();
    let proof = CatalogMutationProofDto::new(
        CatalogMutationProof {
            operation_id: request.request_id,
            conversation_id: ConversationId::from_uuid(request.conversation_id),
            epoch,
            sql_fence: 7,
            replayed: true,
        },
        "agent.queue.preview.cancel",
    );
    use research_agent_core::input_queue::{InputQueue, QueueEdit, QueuedInput};
    let queue = InputQueue::new(request.conversation_id)
        .unwrap()
        .apply(
            0,
            QueueEdit::Add(QueuedInput {
                id: request.input_id,
                text: "retained".into(),
                user_input: vec![],
                origin: None,
            }),
        )
        .unwrap()
        .apply(1, QueueEdit::Prepare(request.input_id))
        .unwrap()
        .apply(2, QueueEdit::Release(request.input_id))
        .unwrap();
    let queue = response(queue, request.manuscript_id).unwrap();
    let status = CancelStatusResponse {
        schema_version: 1,
        intent: request,
        status: CancelStatus::Committed { proof, queue },
    };
    let value = serde_json::to_value(status).unwrap();
    assert_eq!(value["intent"], intent);
    assert_eq!(value["status"]["type"], "committed");
    assert_eq!(
        value["status"]["proof"],
        json!({"schemaVersion":1,"conversationId":intent["conversationId"],"operationId":intent["requestId"],"family":"agent.queue.preview.cancel","epoch":epoch,"sqlFence":"7","replayed":true,"result":null})
    );
}

#[test]
fn storage_failure_keeps_original_identity_and_numeric_safe_cause() {
    let request = request();
    let diagnostic = SqlImmediateDiagnostic {
        operation_id: request.request_id,
        epoch: Some(Uuid::new_v4()),
        sql_fence: Some("7".into()),
        phase: "commit",
        sqlite_code: Some("ConstraintViolation".into()),
        sqlite_extended_code: Some(787),
        safe_cause: "sql_commit_failed",
    };
    let value =
        serde_json::to_value(request.error(SqlImmediateFailure::Unknown(Box::new(diagnostic))))
            .unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["conversationId"],
        request.conversation_id.to_string()
    );
    assert_eq!(
        value["reason"]["manuscriptId"],
        request.manuscript_id.to_string()
    );
    assert_eq!(
        value["reason"]["previewRequestId"],
        request.preview_request_id.to_string()
    );
    assert_eq!(value["reason"]["inputId"], request.input_id.to_string());
    assert_eq!(
        value["reason"]["operationId"],
        request.request_id.to_string()
    );
    assert_eq!(value["reason"]["reason"]["sqliteExtendedCode"], 787);
}

#[test]
fn status_absence_and_uncertainty_do_not_fabricate_a_proof() {
    let request = request();
    let unconfirmed = CancelStatusResponse {
        schema_version: 1,
        intent: request.clone(),
        status: CancelStatus::Unconfirmed {
            detail: Box::new(SqlImmediateDiagnostic {
                operation_id: request.request_id,
                epoch: Some(Uuid::new_v4()),
                sql_fence: Some("0".into()),
                phase: "status",
                sqlite_code: None,
                sqlite_extended_code: None,
                safe_cause: "owner_blocked_receipt_unconfirmed",
            }),
        },
    };
    let value = serde_json::to_value(unconfirmed).unwrap();
    assert_eq!(value["status"]["type"], "unconfirmed");
    assert!(value["status"].get("proof").is_none());
    assert!(value["status"]["detail"]["sqliteCode"].is_null());
    let absent = serde_json::to_value(CancelStatusResponse {
        schema_version: 1,
        intent: request,
        status: CancelStatus::NotFound,
    })
    .unwrap();
    assert_eq!(absent["status"], json!({"type":"notFound"}));
}
