//! Native post-compact fresh-input/steer ordering for both remote contracts.
use super::*;

#[tokio::test]
async fn native_pre_turn_remote_compaction_samples_fresh_input_before_pending_steer() {
    exercise(false).await;
}

#[tokio::test]
async fn full_window_pre_turn_samples_fresh_input_before_pending_steer() {
    exercise(true).await;
}

async fn exercise(full_window: bool) {
    use research_agent_core::steering::{SteeringInput, SteeringScope, SteeringStore};
    let fixture = Fixture::new().await;
    let provider_window = legacy::full_window("pre-turn supplier window");
    let compact = if full_window {
        legacy::json_response(&provider_window)
    } else {
        ok(&events(&[opaque()], true))
    };
    let (url, peer) = sequential_server(vec![ok(&body()), compact, ok(&body()), ok(&body())]).await;
    let first = fixture.record(&"old user evidence ".repeat(900)).await;
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
    let next = fixture.record("fresh submission").await;
    let next_transport = fixture.transport(&next, &url);
    let mut execution = if full_window {
        legacy::legacy_execution(&fixture, &next, &url)
    } else {
        fixture.execution(&next, next_transport.clone())
    };
    // The original history crosses this model's token trigger; the fresh
    // input remains outside the pre-turn compact request.
    accounting::account_for_old_history(&mut execution, next_transport);
    execution.max_input_bytes = usize::MAX;
    let held = Arc::new(HeldCandidate {
        inner: execution.compaction_strategy.take().unwrap(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    execution.compaction_strategy = Some(held.clone());
    let steering = Arc::new(
        crate::agent::repository::steering::SqliteSteeringStore::new(
            fixture.app.shared_agent_connection(),
        ),
    );
    let admission = Arc::new(execution)
        .start_with_steering(
            fixture.service.clone(),
            fixture.store.clone(),
            ConversationId::from_uuid(fixture.id),
            next.request_id,
            Some(steering.clone()),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task")
    };
    timeout(Duration::from_secs(3), held.entered.notified())
        .await
        .unwrap();
    let input_id = Uuid::new_v4();
    steering
        .enqueue(SteeringInput {
            scope: SteeringScope {
                conversation_id: fixture.id,
                turn_id: next.turn_id,
            },
            input_id,
            text: "steer during compact".into(),
            user_input: vec![],
            skill_instructions: vec![],
        })
        .await
        .unwrap();
    held.release.notify_one();
    let outcome = timeout(Duration::from_secs(3), owned.task.wait())
        .await
        .unwrap();
    if outcome != TaskCompletion::Completed {
        let history = fixture
            .service
            .open(ConversationId::from_uuid(fixture.id))
            .await
            .unwrap();
        panic!(
            "synthetic pre-turn failure: {:?}",
            history.turn(next.turn_id).unwrap().failure
        );
    }
    let sent = peer.await.unwrap();
    assert!(!sent[1].to_string().contains("fresh submission"));
    assert!(!sent[1].to_string().contains("steer during compact"));
    assert!(sent[2].to_string().contains("fresh submission"));
    assert!(!sent[2].to_string().contains("steer during compact"));
    assert!(sent[3].to_string().contains("steer during compact"));
    if full_window {
        for request in &sent[2..] {
            assert_eq!(
                &request["input"].as_array().unwrap()[1..1 + provider_window.len()],
                provider_window.as_slice()
            );
            assert!(!request.to_string().contains("old user evidence"));
        }
    } else {
        assert!(sent[2]["input"].as_array().unwrap().contains(&opaque()));
    }
    assert_eq!(
        sent[3]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "compaction")
            .count(),
        1
    );
    let window = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.phase, CompactionPhase::PreTurn);
    let history = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(history.turn(next.turn_id).unwrap().items[&input_id].completed);
}
