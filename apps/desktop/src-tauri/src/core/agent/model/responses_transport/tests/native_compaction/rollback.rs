//! Original-thread rollback: verify actual model requests across installed windows.
use super::*;
use crate::agent::service::ConversationRollbackRequest;

#[tokio::test]
async fn native_v2_rollback_reopens_surviving_history_without_reexecuting_tools() {
    rollback_samples(false, false).await;
}

#[tokio::test]
async fn native_full_window_rollback_reopens_surviving_history_without_reexecuting_tools() {
    rollback_samples(true, false).await;
}

#[tokio::test]
async fn native_v2_rollback_preserves_paired_tool_tail_once() {
    rollback_samples(false, true).await;
}

#[tokio::test]
async fn native_full_window_rollback_preserves_paired_tool_tail_once() {
    rollback_samples(true, true).await;
}

async fn sample(
    fixture: &Fixture,
    url: &str,
    text: &str,
    compact: bool,
    legacy: bool,
    tools: Arc<fork::CountedTool>,
) -> SubmissionRecord {
    let record = fixture.record(text).await;
    let mut execution = if legacy {
        legacy::legacy_execution(fixture, &record, url)
    } else {
        fixture.execution(&record, fixture.transport(&record, url))
    };
    if text.starts_with("after rollback") {
        execution.system_instructions = "rollback current authority".into();
    }
    execution.host_tools = OwnedToolRegistration::new(
        vec![research_agent_core::model::ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized evidence".into(),
            parameters: json!({"type":"object"}),
        }],
        tools,
    );
    assert_eq!(
        fixture
            .start(&record, execution, compact)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    record
}

async fn rollback(fixture: &Fixture, last: Uuid, user_turns: u32) {
    let id = ConversationId::from_uuid(fixture.id);
    let history = fixture.service.open(id).await.unwrap();
    fixture
        .service
        .rollback_conversation(ConversationRollbackRequest {
            conversation_id: id,
            request_id: Uuid::new_v4(),
            expected_watermark: history.seq(),
            expected_last_turn: last,
            user_turns,
        })
        .await
        .unwrap();
}

async fn rollback_samples(legacy: bool, retained_tail: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.sqlite");
    let id = Uuid::new_v4();
    let mut fixture = resume::fixture_at(&path, id);
    fixture
        .service
        .create(ConversationId::from_uuid(id))
        .await
        .unwrap();
    let first = if legacy {
        legacy::full_window("first surviving window")
    } else {
        vec![json!({"type":"compaction","encrypted_content":"first surviving window"})]
    };
    let second = if legacy {
        legacy::full_window("removed second window")
    } else {
        vec![json!({"type":"compaction","encrypted_content":"removed second window"})]
    };
    let compact = |window: &[Value]| {
        if legacy {
            legacy::json_response(window)
        } else {
            ok(&events(window, true))
        }
    };
    let mut replies = vec![
        ok(&body()),
        ok(&fork::tool_response()),
        ok(&body()),
        compact(&first),
        ok(&fork::tool_response_for(
            "removed-later-call",
            json!({"evidence":"removed later evidence"}),
        )),
        ok(&body()),
        compact(&second),
        ok(&body()),
        ok(&body()),
    ];
    if retained_tail {
        replies.insert(
            4,
            ok(&fork::tool_response_for(
                "surviving-tail-call",
                json!({"evidence":"surviving tail evidence"}),
            )),
        );
        replies.insert(5, ok(&body()));
    }
    let (url, peer) = sequential_server(replies).await;
    let tools = Arc::new(fork::CountedTool(AtomicUsize::new(0)));
    let baseline = sample(
        &fixture,
        &url,
        "baseline before windows",
        false,
        legacy,
        tools.clone(),
    )
    .await;
    sample(
        &fixture,
        &url,
        "first user with tools",
        false,
        legacy,
        tools.clone(),
    )
    .await;
    sample(&fixture, &url, "/compact", true, legacy, tools.clone()).await;
    let first_checkpoint = fixture
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    if retained_tail {
        sample(
            &fixture,
            &url,
            "surviving user with tool tail",
            false,
            legacy,
            tools.clone(),
        )
        .await;
    }
    let later = sample(
        &fixture,
        &url,
        "removed later user",
        false,
        legacy,
        tools.clone(),
    )
    .await;
    let second_compact = sample(&fixture, &url, "/compact", true, legacy, tools.clone()).await;
    rollback(&fixture, second_compact.turn_id, 1).await;
    assert_eq!(
        fixture
            .service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        first_checkpoint.window_id
    );
    assert!(
        fixture
            .service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn(later.turn_id)
            .is_none()
    );
    drop(fixture);
    fixture = resume::fixture_at(&path, id);
    let fresh = sample(
        &fixture,
        &url,
        "after rollback surviving window",
        false,
        legacy,
        tools.clone(),
    )
    .await;
    rollback(&fixture, fresh.turn_id, if retained_tail { 3 } else { 2 }).await;
    assert!(
        fixture
            .service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn_order(),
        &[baseline.turn_id]
    );
    drop(fixture);
    fixture = resume::fixture_at(&path, id);
    sample(
        &fixture,
        &url,
        "after rollback original history",
        false,
        legacy,
        tools.clone(),
    )
    .await;
    let sent = peer.await.unwrap();
    let offset = if retained_tail { 2 } else { 0 };
    let restored = 7 + offset;
    let original_history = 8 + offset;
    assert_eq!(sent.len(), 9 + offset);
    assert_eq!(
        tools.0.load(Ordering::SeqCst),
        if retained_tail { 3 } else { 2 }
    );
    let retained = sent[restored]["input"].as_array().unwrap();
    for item in &first {
        assert_eq!(retained.iter().filter(|value| *value == item).count(), 1);
    }
    for index in [restored, original_history] {
        let encoded = sent[index].to_string();
        assert!(!encoded.contains("removed later user"));
        assert!(!encoded.contains("removed second window"));
        assert!(!encoded.contains("historical tool evidence"));
        assert!(!encoded.contains("parent-tool-call"));
        assert!(!encoded.contains("removed-later-call"));
        assert!(!encoded.contains("removed later evidence"));
        assert_eq!(encoded.matches("rollback current authority").count(), 1);
        assert!(!encoded.contains("current research authority"));
        if !retained_tail || index == original_history {
            assert!(
                sent[index]["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["type"] != "function_call"
                        && item["type"] != "function_call_output")
            );
        }
    }
    if retained_tail {
        let calls = retained
            .iter()
            .filter(|item| item["type"] == "function_call")
            .collect::<Vec<_>>();
        let results = retained
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert_eq!(results.len(), 1);
        assert_eq!(calls[0]["call_id"], results[0]["call_id"]);
        assert_eq!(results[0]["output"], "surviving tail evidence");
        assert_eq!(
            sent[restored]
                .to_string()
                .matches("surviving user with tool tail")
                .count(),
            1
        );
    }
    assert!(
        sent[original_history]
            .to_string()
            .contains("baseline before windows")
    );
    assert!(
        !sent[original_history]
            .to_string()
            .contains("first surviving window")
    );
    assert!(
        !sent[original_history]
            .to_string()
            .contains("first user with tools")
    );
    assert!(
        !sent[original_history]
            .to_string()
            .contains("surviving tail evidence")
    );
    assert!(
        !sent[original_history]
            .to_string()
            .contains("surviving user with tool tail")
    );
    assert!(
        !sent[original_history]
            .to_string()
            .contains("after rollback surviving window")
    );
    assert!(
        sent[original_history]["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "compaction")
    );
    let db = fixture.app.shared_agent_connection();
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_context_checkpoints",
                [],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
        2
    );
    let original = fixture
        .service
        .read_since(ConversationId::from_uuid(id), 0, 500)
        .await
        .unwrap();
    assert!(original.items.iter().any(|event| matches!(event.payload(), research_agent_core::protocol::EventPayload::TurnStarted {turn_id,..} if *turn_id == later.turn_id)));
}
