use super::super::CatalogCommandError;
use super::*;
use research_agent_core::conversation::{CatalogDataFailureKind, CatalogMutationDiagnostic};
use serde_json::json;

#[test]
fn request_requires_operation_identity_keeps_raw_text_and_allows_opaque_expected_title() {
    let expected = format!("{}\0", "旧".repeat(200));
    let request = json!({"operationId":Uuid::from_u128(1),"id":Uuid::from_u128(2),"expectedTitle":expected,"title":" B "});
    let command = serde_json::from_value::<RenameRequest>(request.clone())
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(command.expected_title, expected);
    assert_eq!(command.title, " B ");
    let mut missing = request.clone();
    missing.as_object_mut().unwrap().remove("operationId");
    assert!(serde_json::from_value::<RenameRequest>(missing).is_err());
    let mut extra = request;
    extra["extra"] = json!(true);
    assert!(serde_json::from_value::<RenameRequest>(extra).is_err());
}
#[test]
fn proof_has_only_immutable_identity_and_decimal_fence_without_current_metadata() {
    let proof = RenameProofDto::new(
        RenameProof {
            operation_id: Uuid::from_u128(1),
            conversation_id: ConversationId::from_uuid(Uuid::from_u128(2)),
            epoch: Uuid::from_u128(3),
            sql_fence: u64::MAX,
            replayed: true,
        },
        FAMILY,
    );
    let value = serde_json::to_value(proof).unwrap();
    assert_eq!(
        value,
        json!({"schemaVersion":1,"operationId":Uuid::from_u128(1),"conversationId":Uuid::from_u128(2),"family":"agent.conversation.rename","epoch":Uuid::from_u128(3),"sqlFence":"18446744073709551615","replayed":true,"result":null})
    );
    let status = RenameStatusDto::new(
        RenameStatus::Committed(RenameProof {
            operation_id: Uuid::from_u128(1),
            conversation_id: ConversationId::from_uuid(Uuid::from_u128(2)),
            epoch: Uuid::from_u128(3),
            sql_fence: u64::MAX,
            replayed: true,
        }),
        FAMILY,
    );
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        json!({"type":"committed","proof":value})
    );
}
#[test]
fn data_diagnostic_preserves_existing_sql_failure_wire_and_domain_literals() {
    let diagnostic = CatalogMutationDiagnostic {
        operation_id: Uuid::from_u128(1),
        epoch: Some(Uuid::from_u128(3)),
        sql_fence: Some("18446744073709551615".into()),
        phase: "commit",
        sqlite_code: Some("SystemIoFailure".into()),
        sqlite_extended_code: Some(10),
        safe_cause: "sql_commit_result_unknown",
    };
    let native = crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
        crate::data_interface::SqlImmediateDiagnostic {
            operation_id: diagnostic.operation_id,
            epoch: diagnostic.epoch,
            sql_fence: diagnostic.sql_fence.clone(),
            phase: diagnostic.phase,
            sqlite_code: diagnostic.sqlite_code.clone(),
            sqlite_extended_code: diagnostic.sqlite_extended_code,
            safe_cause: diagnostic.safe_cause,
        },
    ));
    let error = RenameCommandError::Data(CatalogDataFailure {
        kind: CatalogDataFailureKind::Unknown,
        diagnostic: Box::new(diagnostic),
    });
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({"code":"storage","reason":serde_json::to_value(native).unwrap()})
    );
    assert_eq!(
        serde_json::to_value(RenameCommandError::Domain(CatalogCommandError::Conflict)).unwrap(),
        json!("conflict")
    );
}
