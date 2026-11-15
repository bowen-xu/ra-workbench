use super::*;
use research_agent_core::conversation::{CatalogFuture, ConversationBinding, ConversationSummary};
use std::sync::Mutex;
struct Reader {
    calls: Mutex<Vec<(ConversationId, ConversationBinding)>>,
    origin: Option<ConversationForkOrigin>,
}
impl ConversationForkReader for Reader {
    fn read_origin(
        &self,
        child: ConversationId,
        binding: ConversationBinding,
    ) -> CatalogFuture<'_, Option<ConversationForkOrigin>> {
        self.calls.lock().unwrap().push((child, binding));
        Box::pin(async { Ok(self.origin.clone()) })
    }
}
fn origin() -> ConversationForkOrigin {
    ConversationForkOrigin {
        parent: ConversationSummary {
            internal: false,
            id: ConversationId::from_uuid(Uuid::from_u128(1)),
            title: "研究对话".into(),
            archived: true,
            durable_count: 8,
            created_at: "2026-09-10T00:00:00.000Z".into(),
            updated_at: "2026-09-10T00:00:00.000Z".into(),
        },
        through_turn_id: Uuid::from_u128(3),
        parent_end_seq: 5,
    }
}
fn request() -> serde_json::Value {
    serde_json::json!({"conversationId":Uuid::from_u128(2),"binding":{"kind":"note","resourceId":"note"}})
}
#[tokio::test]
async fn wire_matches_fixture_and_roots_are_explicitly_null() {
    let reader = Reader {
        calls: Mutex::new(vec![]),
        origin: Some(origin()),
    };
    let result = execute(&reader, serde_json::from_value(request()).unwrap())
        .await
        .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent/conversation-fork-origin.json"
    )))
    .unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), expected);
    assert_eq!(reader.calls.lock().unwrap()[0].1.resource_id, "note");
    let root = Reader {
        calls: Mutex::new(vec![]),
        origin: None,
    };
    assert!(
        execute(&root, serde_json::from_value(request()).unwrap())
            .await
            .unwrap()
            .origin
            .is_none()
    );
}
#[tokio::test]
async fn rejects_invalid_requests_before_reading_and_invalid_origins_afterward() {
    let reader = Reader {
        calls: Mutex::new(vec![]),
        origin: None,
    };
    let mut bad = request();
    bad["conversationId"] = serde_json::json!(Uuid::nil());
    assert!(matches!(
        execute(&reader, serde_json::from_value(bad).unwrap()).await,
        Err(CatalogCommandError::InvalidInput)
    ));
    let mut bad = request();
    bad["binding"]["resourceId"] = "".into();
    assert!(
        execute(&reader, serde_json::from_value(bad).unwrap())
            .await
            .is_err()
    );
    assert!(reader.calls.lock().unwrap().is_empty());
    let mut bad = request();
    bad["execute"] = true.into();
    assert!(serde_json::from_value::<OriginRequest>(bad).is_err());
    let mut own = origin();
    own.parent.id = ConversationId::from_uuid(Uuid::from_u128(2));
    let reader = Reader {
        calls: Mutex::new(vec![]),
        origin: Some(own),
    };
    assert!(matches!(
        execute(&reader, serde_json::from_value(request()).unwrap()).await,
        Err(CatalogCommandError::Corrupt)
    ));
    for end in [0, 9, u64::MAX] {
        let mut bad = origin();
        bad.parent_end_seq = end;
        assert!(OriginDto::try_from(bad).is_err());
    }
}
