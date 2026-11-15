use super::*;
use crate::data_interface::SqlImmediateDiagnostic;
use serde_json::json;

#[test]
fn identity_fields_are_required_even_when_their_value_is_explicitly_null() {
    let source = json!({"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"text":"/compact",
        "context":{"manuscriptId":Uuid::new_v4(),"files":[]},"model":null});
    let prepare = json!({"inputId":Uuid::new_v4(),"expectedQueueRevision":0,"previousRequestId":null,
        "submission":source,"expectedRevision":0});
    assert!(serde_json::from_value::<Prepare>(prepare.clone()).is_ok());
    let mut missing = prepare;
    missing.as_object_mut().unwrap().remove("previousRequestId");
    assert!(serde_json::from_value::<Prepare>(missing).is_err());
    let read = json!({"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"inputId":Uuid::new_v4(),"requestId":null});
    assert!(serde_json::from_value::<Read>(read.clone()).is_ok());
    let mut missing = read;
    missing.as_object_mut().unwrap().remove("requestId");
    assert!(serde_json::from_value::<Read>(missing).is_err());
}

#[test]
fn settlement_accepts_only_the_frozen_identity_and_has_no_renderer_outcome_or_remove_action() {
    let value = json!({"operationId":Uuid::new_v4(),"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),
        "inputId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"expectedQueueRevision":3,"claimRevision":2});
    assert!(serde_json::from_value::<Settle>(value.clone()).is_ok());
    for field in ["outcome", "action", "text", "expectedRevision", "execute"] {
        let mut bad = value.clone();
        bad[field] = json!("unexpected");
        assert!(serde_json::from_value::<Settle>(bad).is_err());
    }
}

#[test]
fn unknown_storage_failures_keep_the_original_c_r_operation_and_safe_driver_details() {
    let conversation = Uuid::new_v4();
    let request = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let error = Error::data(
        SqlImmediateFailure::Unknown(Box::new(SqlImmediateDiagnostic {
            operation_id: operation,
            epoch: None,
            sql_fence: None,
            phase: "commit",
            sqlite_code: Some("ConstraintViolation".into()),
            sqlite_extended_code: Some(787),
            safe_cause: "sql_commit_outcome_unknown",
        })),
        conversation,
        Some(request),
        operation,
    );
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(value["reason"]["conversationId"], conversation.to_string());
    assert_eq!(value["reason"]["requestId"], request.to_string());
    assert_eq!(value["reason"]["operationId"], operation.to_string());
    assert_eq!(value["reason"]["reason"]["sqliteExtendedCode"], 787);
}

#[test]
fn discovery_omits_retirement_until_present_and_preserves_its_boxed_wire_shape() {
    use crate::data_interface::{CancelPreviewIntent, QueuePreviewRetirement, SqlImmediateReceipt};
    use research_agent_core::{
        input_queue::{InputQueue, QueueEdit, QueuedInput},
        protocol::ModelIdentity,
        submission::{SubmissionInput, SubmissionRecord},
    };
    let conversation = Uuid::new_v4();
    let manuscript = Uuid::new_v4();
    let input = Uuid::new_v4();
    let request = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let epoch = Uuid::new_v4();
    let queue = InputQueue::new(conversation)
        .unwrap()
        .apply(
            0,
            QueueEdit::Add(QueuedInput {
                id: input,
                text: "/compact".into(),
                user_input: vec![],
                origin: None,
            }),
        )
        .unwrap()
        .apply(1, QueueEdit::Prepare(input))
        .unwrap();
    let mut state = QueuedCompactionState {
        record: SubmissionRecord {
            conversation_id: conversation,
            request_id: request,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            input: SubmissionInput {
                remote_compaction_v2: None,
                text: "/compact".into(),
                user_input: vec![],
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "synthetic".into(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                skill_instructions: vec![],
            },
        },
        queue: queue.clone(),
        claim_revision: 1,
        expected_revision: 0,
        outcome: None,
        current: true,
        retirement: None,
    };
    let original = serde_json::to_value(Discovery::Compact(
        preview(state.clone(), manuscript, input).unwrap(),
    ))
    .unwrap();
    assert!(original.get("retirement").is_none());
    state.current = false;
    // This fixture exercises only serialization. Typed proof admission is
    // covered by the actual Database retirement tests.
    state.retirement = Some(Box::new(QueuePreviewRetirement {
        intent: CancelPreviewIntent {
            conversation_id: conversation,
            manuscript_id: manuscript,
            request_id: operation,
            expected_queue_revision: 2,
            input_id: input,
            preview_request_id: request,
        },
        queue: queue.apply(2, QueueEdit::Release(input)).unwrap(),
        receipt: SqlImmediateReceipt {
            operation_id: operation,
            epoch,
            sql_fence: research_data_runtime::ReadFence { epoch, position: 7 },
            fingerprint: [0; 32],
            result: (),
            replayed: true,
        },
    }));
    let retired = serde_json::to_value(Discovery::Compact(
        preview(state, manuscript, input).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        retired["retirement"]["intent"]["previewRequestId"],
        request.to_string()
    );
    assert_eq!(
        retired["retirement"]["status"]["proof"],
        json!({
            "schemaVersion": 1, "operationId": operation, "conversationId": conversation,
            "family": "agent.queue.preview.cancel", "epoch": epoch, "sqlFence": "7", "replayed": true, "result": null,
        })
    );
    assert_eq!(retired["queue"]["revision"], 2);
    assert_eq!(retired["retirement"]["status"]["queue"]["revision"], 3);
    assert_eq!(retired["outcome"], serde_json::Value::Null);
}
