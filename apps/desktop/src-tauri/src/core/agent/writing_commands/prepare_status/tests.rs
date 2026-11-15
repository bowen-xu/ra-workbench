use super::*;
use crate::data_interface::SqlImmediateReceipt;
use research_data_runtime::ReadFence;
use serde_json::Value;

fn id(value: &Value) -> Uuid {
    serde_json::from_value(value.clone()).unwrap()
}
fn committed(request: &PrepareRequest, result: &Value, proof: &Value) -> WritingPrepareStatus {
    assert_eq!(id(&proof["operationId"]), request.request_id);
    assert_eq!(id(&proof["conversationId"]), request.conversation_id);
    assert_eq!(proof["family"], WRITING_PREPARE_FAMILY);
    assert_eq!(proof["schemaVersion"], 1);
    assert_eq!(proof["result"], Value::Null);
    let input: SubmissionInput = serde_json::from_value(result["input"].clone()).unwrap();
    input.validate().unwrap();
    assert!(input.context.iter().any(|source| {
        crate::agent::context_session::resource_grants::resource(source).is_some()
    }));
    let record = SubmissionRecord {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        turn_id: id(&result["turnId"]),
        context_snapshot_id: id(&result["contextSnapshotId"]),
        input,
    };
    WritingPrepareStatus::Committed {
        record: Box::new(record),
        receipt: Box::new(SqlImmediateReceipt {
            operation_id: id(&proof["operationId"]),
            epoch: id(&proof["epoch"]),
            sql_fence: ReadFence {
                epoch: id(&proof["epoch"]),
                position: proof["sqlFence"].as_str().unwrap().parse().unwrap(),
            },
            fingerprint: [0; 32],
            result: (),
            replayed: proof["replayed"].as_bool().unwrap(),
        }),
    }
}
fn diagnostic(
    value: &Value,
    phase: &'static str,
    safe_cause: &'static str,
) -> SqlImmediateDiagnostic {
    assert_eq!(value.as_object().unwrap().len(), 7);
    assert_eq!(value["phase"], phase);
    assert_eq!(value["safeCause"], safe_cause);
    SqlImmediateDiagnostic {
        operation_id: id(&value["operationId"]),
        epoch: serde_json::from_value(value["epoch"].clone()).unwrap(),
        sql_fence: serde_json::from_value(value["sqlFence"].clone()).unwrap(),
        phase,
        sqlite_code: serde_json::from_value(value["sqliteCode"].clone()).unwrap(),
        sqlite_extended_code: serde_json::from_value(value["sqliteExtendedCode"].clone()).unwrap(),
        safe_cause,
    }
}

#[test]
fn shared_native_prepare_fixture_matches_typed_intents_status_proof_and_diagnostic_serializers() {
    let fixtures: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent-writing-prepare-native-v1.json"
    )))
    .unwrap();
    let ordinary = &fixtures["ordinary"];
    let request: PrepareRequest = serde_json::from_value(ordinary["intent"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), ordinary["intent"]);
    let status = status_value(
        &request,
        committed(
            &request,
            &ordinary["status"]["result"],
            &ordinary["status"]["proof"],
        ),
        |record| SubmissionPreview::from_record(record, request.context.manuscript_id),
    );
    assert_eq!(
        serde_json::to_value(PrepareStatusResponse {
            schema_version: 1,
            intent: request,
            status
        })
        .unwrap(),
        *ordinary
    );

    let compact = &fixtures["compact"];
    let intent: crate::agent::compaction_commands::preparation::Prepare =
        serde_json::from_value(compact["intent"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&intent).unwrap(), compact["intent"]);
    assert_eq!(
        compact["status"]["result"]["expectedRevision"],
        intent.expected_revision
    );
    let value = committed(
        &intent.request,
        &compact["status"]["result"]["preview"],
        &compact["status"]["proof"],
    );
    assert_eq!(
        serde_json::to_value(
            crate::agent::compaction_commands::preparation::PrepareStatusResponse::from_status(
                intent, value
            )
        )
        .unwrap(),
        *compact
    );

    let unconfirmed = &fixtures["unconfirmed"];
    let request: PrepareRequest = serde_json::from_value(unconfirmed["intent"].clone()).unwrap();
    let detail = diagnostic(
        &unconfirmed["status"]["detail"],
        "writing-prepare-status",
        "legacy_prepare_without_intent_proof",
    );
    assert_eq!(detail.operation_id, request.request_id);
    let status = status_value(
        &request,
        WritingPrepareStatus::Unconfirmed(Box::new(detail)),
        |record| SubmissionPreview::from_record(record, request.context.manuscript_id),
    );
    assert_eq!(
        serde_json::to_value(PrepareStatusResponse {
            schema_version: 1,
            intent: request.clone(),
            status
        })
        .unwrap(),
        *unconfirmed
    );

    let storage = &fixtures["storageFailure"];
    let detail = diagnostic(
        &storage["reason"]["reason"],
        "status",
        "sql_registration_failed",
    );
    assert_eq!(detail.operation_id, request.request_id);
    assert_eq!(detail.sqlite_code.as_deref(), Some("Unknown"));
    assert_eq!(detail.sqlite_extended_code, Some(1));
    assert_eq!(
        serde_json::to_value(
            request.storage_failure(SqlImmediateFailure::Unavailable(Box::new(detail)))
        )
        .unwrap(),
        *storage
    );
}
