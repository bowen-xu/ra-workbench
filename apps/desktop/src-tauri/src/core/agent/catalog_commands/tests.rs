use super::*;
#[test]
fn activity_wire_exposes_only_ids_and_classification_and_rejects_extra_capabilities() {
    assert!(
        serde_json::from_value::<ActivityRequest>(serde_json::json!({}))
            .unwrap()
            .binding
            .is_none()
    );
    assert!(serde_json::from_value::<ActivityRequest>(serde_json::json!({"start":true})).is_err());
    let response = ActivityResponse {
        schema_version: 1,
        running: vec![super::super::repository::ConversationActivity {
            id: Uuid::from_u128(1),
            internal: true,
        }],
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({"schemaVersion":1,"running":[{"id":Uuid::from_u128(1),"internal":true}]})
    );
}
#[test]
fn exact_metadata_request_exposes_only_identity_and_optional_binding() {
    let request: ReadRequest = serde_json::from_value(serde_json::json!({"id":Uuid::from_u128(1), "binding":{"kind":"manuscript", "resourceId":"one"}})).unwrap();
    assert_eq!(
        request.binding.unwrap().decode().unwrap().resource_id,
        "one"
    );
    for value in [
        serde_json::json!({"id":"bad"}),
        serde_json::json!({"id":Uuid::from_u128(1), "archived":false}),
        serde_json::json!({"id":Uuid::from_u128(1), "binding":{"kind":"unknown", "resourceId":"one"}}),
    ] {
        assert!(serde_json::from_value::<ReadRequest>(value).is_err());
    }
}
#[test]
fn archive_requests_use_explicit_boolean_state_and_reject_extra_capabilities() {
    for archived in [true, false] {
        let request: ArchiveRequest = serde_json::from_value(
            serde_json::json!({"operationId":Uuid::from_u128(2),"id":Uuid::from_u128(1), "archived":archived,"expectedWatermark":archived.then_some(0)}),
        )
        .unwrap();
        assert_eq!(request.id, Uuid::from_u128(1));
        assert_eq!(request.archived, archived);
    }
    for value in [
        serde_json::json!({"operationId":Uuid::from_u128(2),"id":"invalid", "archived":true,"expectedWatermark":0}),
        serde_json::json!({"operationId":Uuid::from_u128(2),"id":Uuid::from_u128(1), "archived":1,"expectedWatermark":0}),
        serde_json::json!({"operationId":Uuid::from_u128(2),"id":Uuid::from_u128(1), "archived":true,"expectedWatermark":0, "stopTask":true}),
    ] {
        assert!(serde_json::from_value::<ArchiveRequest>(value).is_err());
    }
}
#[test]
fn metadata_wire_matches_shared_fixture_and_rejects_lossy_counts() {
    let summary = ConversationSummary {
        internal: false,
        id: ConversationId::from_uuid(Uuid::from_u128(1)),
        title: "研究对话".into(),
        archived: false,
        durable_count: 4,
        created_at: "2026-09-08T00:00:00.000Z".into(),
        updated_at: "2026-09-08T01:00:00.000Z".into(),
    };
    let response = ListResponse {
        schema_version: 1,
        conversations: vec![SummaryDto::try_from(summary.clone()).unwrap()],
        next: Some(CursorDto {
            created_at: summary.created_at.clone(),
            id: *summary.id.as_uuid(),
        }),
    };
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent-catalog-v1.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
    assert!(
        SummaryDto::try_from(ConversationSummary {
            internal: false,
            durable_count: 9_007_199_254_740_992,
            ..summary
        })
        .is_err()
    );
}
#[test]
fn metadata_requests_reject_unknown_fields_invalid_ids_and_unbounded_pages() {
    assert!(
        serde_json::from_value::<RenameRequest>(
            serde_json::json!({"id":"wrong","expectedTitle":"a","title":"b"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ListRequest>(
            serde_json::json!({"archived":false,"limit":2,"before":null,"extra":true})
        )
        .is_err()
    );
    for limit in [0, 501] {
        assert!(
            ListRequest {
                internal: false,
                binding: None,
                archived: false,
                limit,
                before: None
            }
            .query()
            .is_err()
        );
    }
    assert!(
        ListRequest {
            internal: false,
            binding: None,
            archived: false,
            limit: 2,
            before: Some(CursorDto {
                created_at: "unexpected".into(),
                id: Uuid::nil()
            })
        }
        .query()
        .is_err()
    );
}

#[test]
fn binding_requests_validate_kind_identity_and_optional_filter() {
    for kind in ["manuscript", "paper", "note"] {
        let request: ListRequest = serde_json::from_value(serde_json::json!({"archived":false,"limit":10,"before":null,"binding":{"kind":kind,"resourceId":"resource"}})).unwrap();
        assert_eq!(
            request.query().unwrap().binding.unwrap().resource_id,
            "resource"
        );
    }
    assert!(
        serde_json::from_value::<BindingDto>(serde_json::json!({"kind":"shell","resourceId":"x"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<BindingDto>(serde_json::json!({"kind":"note","resourceId":""}))
            .unwrap()
            .decode()
            .is_err()
    );
    let unfiltered: ListRequest =
        serde_json::from_value(serde_json::json!({"archived":false,"limit":10,"before":null}))
            .unwrap();
    assert!(unfiltered.query().unwrap().binding.is_none());
}

#[test]
fn catalog_scope_defaults_to_user_and_explicit_internal_scope_is_preserved() {
    for internal in [None, Some(false), Some(true)] {
        let mut value = serde_json::json!({"archived":false,"limit":10,"before":null});
        if let Some(internal) = internal {
            value["internal"] = internal.into();
        }
        let request: ListRequest = serde_json::from_value(value).unwrap();
        assert_eq!(request.query().unwrap().internal, internal.unwrap_or(false));
    }
    assert!(
        serde_json::from_value::<ListRequest>(
            serde_json::json!({"archived":false,"limit":10,"before":null,"internal":"true"})
        )
        .is_err()
    );
}
