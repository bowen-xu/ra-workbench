use super::*;
use research_agent_core::conversation::{
    ConversationId, ConversationSearchHit, ConversationSummary,
};

#[test]
fn wire_matches_shared_search_fixture() {
    let timestamp = "2026-09-10T00:00:00.000Z".to_owned();
    let response = SearchResponse::try_from(ConversationSearchPage {
        hits: vec![ConversationSearchHit {
            conversation: ConversationSummary {
                internal: false,
                id: ConversationId::from_uuid(Uuid::from_u128(1)),
                title: "研究对话".into(),
                archived: false,
                durable_count: 13,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            },
            excerpt: "Saved partial response 跨片段🙂".into(),
            item_id: Some(Uuid::from_u128(14)),
            turn_id: Some(Uuid::from_u128(2)),
            start_seq: Some(12),
            completed: false,
        }],
        next: None,
    })
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent/conversation-search.json"
    )))
    .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
}
#[test]
fn request_rejects_unknown_capabilities_and_invalid_scope() {
    for value in [
        serde_json::json!({"term":"text", "scope":{"archived":false,"limit":1}, "includeSecrets":true}),
        serde_json::json!({"term":"text", "scope":{"archived":false,"limit":1,"open":true}}),
        serde_json::json!({"term":"text", "scope":{"archived":false,"limit":1,"binding":{"kind":"shell","resourceId":"one"}}}),
    ] {
        assert!(serde_json::from_value::<SearchRequest>(value).is_err());
    }
    let request: SearchRequest =
        serde_json::from_value(serde_json::json!({"term":"text", "scope":{
        "archived":false,"limit":1,"before":{"createdAt":"bad","id":Uuid::from_u128(1)}}}))
        .unwrap();
    assert!(request.scope.query().is_err());
}

#[test]
fn response_rejects_anchors_outside_the_durable_history() {
    let hit = ConversationSearchHit {
        conversation: ConversationSummary {
            internal: false,
            id: ConversationId::from_uuid(Uuid::from_u128(1)),
            title: "Title".into(),
            archived: false,
            durable_count: 10,
            created_at: "2026-09-10T00:00:00.000Z".into(),
            updated_at: "2026-09-10T00:00:00.000Z".into(),
        },
        excerpt: "Message".into(),
        item_id: Some(Uuid::from_u128(2)),
        turn_id: Some(Uuid::from_u128(3)),
        start_seq: Some(11),
        completed: false,
    };
    for invalid in [
        hit.clone(),
        ConversationSearchHit {
            start_seq: Some(0),
            ..hit.clone()
        },
        ConversationSearchHit {
            start_seq: Some(1),
            item_id: None,
            ..hit.clone()
        },
        ConversationSearchHit {
            start_seq: Some(1),
            excerpt: "🙂".repeat(321),
            ..hit
        },
    ] {
        assert!(matches!(
            SearchResponse::try_from(ConversationSearchPage {
                hits: vec![invalid],
                next: None
            }),
            Err(CatalogCommandError::Corrupt)
        ));
    }
}
