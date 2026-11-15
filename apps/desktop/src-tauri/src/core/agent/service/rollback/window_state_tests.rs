use super::tests::{finish, model, setup, start};
use super::*;
use crate::agent::{
    cancellation::AgentCancellation, repository::context_windows::identity::WindowIdentity,
};
use research_agent_core::context::{
    checkpoint::ContextCheckpoint,
    checkpoint_store::{CheckpointInstallation, CheckpointStoreError},
    compaction_strategy::*,
    token_budget::*,
};

async fn claim(
    service: &AgentSessionService,
    id: ConversationId,
    turn: Uuid,
    window: WindowIdentity,
) -> bool {
    let slot = service.slot(id, false).unwrap();
    let mut owner = slot.lock().await;
    owner
        .as_mut()
        .unwrap()
        .post_sampling_token_budget(
            turn,
            window.current,
            &TokenBudgetPolicy::resolve(TokenBudgetSettings {
                reminder_threshold_tokens: Some(2),
                ..Default::default()
            })
            .unwrap(),
            TokenBudgetStatus {
                active_context_tokens: Some(99),
                auto_compact_scope_tokens: Some(99),
                auto_compact_scope_limit: Some(100),
                full_context_window_limit: Some(200),
                base_window_tokens_remaining: Some(1),
                full_context_window_limit_reached: false,
                token_limit_reached: false,
            },
            false,
        )
        .unwrap()
        .reminder
        .is_some()
}

#[tokio::test]
async fn install_replay_and_tail_rollback_preserve_claims_but_window_rollback_reconciles() {
    let (_db, service, id) = setup();
    service.create(id).await.unwrap();
    let first = start(&service, id).await;
    let initial = service
        .prepare_token_budget_window(*id.as_uuid(), first)
        .await
        .unwrap();
    let checkpoint = ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::now_v7(),
        conversation_id: *id.as_uuid(),
        turn_id: first,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model: model(),
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("trusted state".into()),
        retained: vec![],
        canonical_context: None,
    };
    assert_eq!(
        service
            .install_context_checkpoint(checkpoint.clone(), 1, AgentCancellation::default())
            .await
            .unwrap(),
        CheckpointInstallation::Installed
    );
    let installed = service
        .prepare_token_budget_window(*id.as_uuid(), first)
        .await
        .unwrap();
    assert_eq!(installed.number, 1);
    assert_eq!(installed.previous, Some(initial.current));
    let mut invalid = checkpoint.clone();
    invalid.previous_window_id = Some(checkpoint.window_id);
    invalid.window_id = initial.current;
    assert_eq!(
        service
            .install_context_checkpoint(invalid, 1, AgentCancellation::default())
            .await,
        Err(CheckpointStoreError::Invalid)
    );
    assert_eq!(
        service
            .prepare_token_budget_window(*id.as_uuid(), first)
            .await
            .unwrap(),
        installed
    );
    assert!(claim(&service, id, first, installed).await);
    service
        .new_context_request(*id.as_uuid(), first, false)
        .await
        .unwrap();
    assert_eq!(
        service
            .install_context_checkpoint(checkpoint, 1, AgentCancellation::default())
            .await
            .unwrap(),
        CheckpointInstallation::AlreadyInstalled { active: true }
    );
    assert!(
        service
            .new_context_request(*id.as_uuid(), first, true)
            .await
            .unwrap()
    );
    assert!(!claim(&service, id, first, installed).await);
    finish(&service, id, first).await;
    let second = start(&service, id).await;
    finish(&service, id, second).await;
    let watermark = service.open(id).await.unwrap().seq();
    service
        .rollback_conversation(ConversationRollbackRequest {
            conversation_id: id,
            request_id: Uuid::new_v4(),
            expected_watermark: watermark,
            expected_last_turn: second,
            user_turns: 1,
        })
        .await
        .unwrap();
    let third = start(&service, id).await;
    {
        let slot = service.slot(id, false).unwrap();
        let mut owner = slot.lock().await;
        let session = owner.as_mut().unwrap();
        assert!(session.request_new_context_window(third).is_err());
        assert!(session.take_new_context_window_request(third).is_err());
    }
    let same = service
        .prepare_token_budget_window(*id.as_uuid(), third)
        .await
        .unwrap();
    assert_eq!(same, installed);
    assert!(!claim(&service, id, third, same).await);
    finish(&service, id, third).await;
    let watermark = service.open(id).await.unwrap().seq();
    service
        .rollback_conversation(ConversationRollbackRequest {
            conversation_id: id,
            request_id: Uuid::new_v4(),
            expected_watermark: watermark,
            expected_last_turn: third,
            user_turns: 2,
        })
        .await
        .unwrap();
    let fourth = start(&service, id).await;
    let restored = service
        .prepare_token_budget_window(*id.as_uuid(), fourth)
        .await
        .unwrap();
    assert_eq!(restored, initial);
    assert!(claim(&service, id, fourth, restored).await);
}
