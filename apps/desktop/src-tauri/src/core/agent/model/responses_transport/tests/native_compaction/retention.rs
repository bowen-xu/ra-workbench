use super::*;
use research_agent_core::context::{
    checkpoint::{StoredContent, StoredMessage},
    retained_text::Utf8RetainedTextPolicy,
};

#[tokio::test]
async fn native_v2_retains_truncated_user_text_in_checkpoint_and_next_request_only() {
    check_retention("HEADmiddleTAIL".into(), 2, None).await;
}

#[tokio::test]
async fn native_v2_byte_guard_keeps_newest_user_and_original_evidence() {
    check_retention(
        format!("HEAD{}TAIL", "中文\n\"".repeat(1_000)),
        64_000,
        Some(1_024),
    )
    .await;
}

async fn check_retention(original: String, max_tokens: u64, byte_limit: Option<usize>) {
    let fixture = Fixture::new().await;
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        ok(&events(&[opaque()], true)),
        ok(&body()),
    ])
    .await;
    let first = fixture.record(&original).await;
    let execution = fixture.execution(&first, fixture.transport(&first, &url));
    assert_eq!(
        fixture
            .start(&first, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let compact = fixture.record("/compact").await;
    let transport = fixture.transport(&compact, &url);
    let mut execution = fixture.execution(&compact, transport.clone());
    if let Some(limit) = byte_limit {
        execution.max_input_bytes = limit;
    }
    let binding = execution.compaction_strategy.as_ref().unwrap().binding();
    execution.compaction_strategy = Some(Arc::new(
        ResponsesV2CompactionStrategy::new(transport, binding)
            .unwrap()
            .with_retained_text_policy(Arc::new(Utf8RetainedTextPolicy { max_tokens }))
            .unwrap(),
    ));
    assert_eq!(
        fixture
            .start(&compact, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let checkpoint = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    let [
        StoredMessage::User {
            source_item_id: _,
            content,
        },
    ] = checkpoint.retained.as_slice()
    else {
        panic!("latest user must remain")
    };
    let [StoredContent::Text { text: retained }] = content.as_slice() else {
        panic!("text expected")
    };
    assert!(retained.starts_with("HEAD") && retained.ends_with("TAIL"));
    assert_eq!(retained.matches("tokens truncated").count(), 1);
    if let Some(limit) = byte_limit {
        assert!(retained.len() <= limit / 4);
    } else {
        assert_eq!(retained, "HEAD…2 tokens truncated…TAIL");
    }
    let next = fixture.record("continue").await;
    let execution = fixture.execution(&next, fixture.transport(&next, &url));
    assert_eq!(
        fixture
            .start(&next, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let projection = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(
        projection
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| item.text == original)
    );
    let requests = peer.await.unwrap();
    // Compare encoded strings so control characters do not invalidate the check.
    let encoded_original = serde_json::to_string(&original).unwrap();
    let encoded_retained = serde_json::to_string(retained).unwrap();
    assert!(requests[1].to_string().contains(&encoded_original));
    assert!(requests[2].to_string().contains(&encoded_retained));
    assert!(!requests[2].to_string().contains(&encoded_original));
    assert_eq!(
        requests[2]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| **item == opaque())
            .count(),
        1
    );
}
