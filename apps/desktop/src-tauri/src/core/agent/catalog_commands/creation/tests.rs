use super::*;
use research_agent_core::conversation::{BindingKind, CatalogMutationProof, CatalogMutationStatus};
use serde_json::json;

#[test]
fn request_requires_independent_operation_identity_and_preserves_raw_binding() {
    let value = json!({"operationId":Uuid::from_u128(1),"id":Uuid::from_u128(2),"binding":{"kind":"readingNote","resourceId":" original "}});
    let command = serde_json::from_value::<CreateBoundRequest>(value.clone())
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(command.binding.kind, BindingKind::ReadingNote);
    assert_eq!(command.binding.resource_id, " original ");
    for field in ["operationId", "id", "binding"] {
        let mut invalid = value.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<CreateBoundRequest>(invalid).is_err());
    }
    let mut extra = value;
    extra["force"] = json!(true);
    assert!(serde_json::from_value::<CreateBoundRequest>(extra).is_err());
}
#[test]
fn proof_and_status_use_creation_family_and_exact_decimal_fence_without_metadata() {
    let proof = CatalogMutationProof {
        operation_id: Uuid::from_u128(1),
        conversation_id: ConversationId::from_uuid(Uuid::from_u128(2)),
        epoch: Uuid::from_u128(3),
        sql_fence: u64::MAX,
        replayed: true,
    };
    let wire = serde_json::to_value(CreateBoundProofDto::new(proof.clone(), FAMILY)).unwrap();
    assert_eq!(
        wire,
        json!({"schemaVersion":1,"operationId":Uuid::from_u128(1),"conversationId":Uuid::from_u128(2),"family":"agent.conversation.create_bound","epoch":Uuid::from_u128(3),"sqlFence":"18446744073709551615","replayed":true,"result":null})
    );
    let status = CreateBoundStatusResponse::new(
        proof.operation_id,
        *proof.conversation_id.as_uuid(),
        CatalogMutationStatus::Committed(proof),
        FAMILY,
    );
    assert_eq!(
        serde_json::to_value(status).unwrap()["status"],
        json!({"type":"committed","proof":wire})
    );
}
