use super::*;
use research_agent_core::context::token_budget::{
    TokenBudgetPolicy, TokenBudgetSettings, TokenBudgetStatus,
};

fn policy() -> TokenBudgetPolicy {
    TokenBudgetPolicy::resolve(TokenBudgetSettings {
        reminder_threshold_tokens: Some(20),
        reminder_message_template: "remaining {n_remaining}".into(),
        auto_compact_fallback_prompt: Some("save progress".into()),
        auto_compact_fallback_buffer_tokens: Some(50),
        ..Default::default()
    })
    .unwrap()
}
fn status() -> TokenBudgetStatus {
    TokenBudgetStatus {
        active_context_tokens: Some(100),
        auto_compact_scope_tokens: Some(100),
        auto_compact_scope_limit: Some(100),
        full_context_window_limit: Some(200),
        base_window_tokens_remaining: Some(0),
        full_context_window_limit_reached: false,
        token_limit_reached: false,
    }
}

#[tokio::test]
async fn persisted_claim_requires_acknowledgement_and_ambiguous_writes_fence_the_owner() {
    use std::{future::Future, task::Poll};
    for outcome in ["success", "error", "cancel"] {
        let (_, _, service, id) = setup();
        service.create(id).await.unwrap();
        service.commit(id, vec![start(id)]).await.unwrap();
        let slot = service.slot(id, false).unwrap();
        let mut owner = slot.lock().await;
        let session = owner.as_mut().unwrap();
        let turn = Uuid::from_u128(2);
        let window = Uuid::from_u128(100);
        session
            .restore_token_budget_window(turn, 0, window, None, window)
            .unwrap();
        let prepared = session
            .prepare_post_sampling_token_budget(turn, window, &policy(), status(), false)
            .unwrap();
        if outcome == "cancel" {
            let mut write = Box::pin(session.persist_post_sampling_token_budget(
                turn,
                prepared,
                std::future::pending(),
            ));
            std::future::poll_fn(|cx| {
                assert!(write.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(write);
        } else {
            let result = session
                .persist_post_sampling_token_budget(turn, prepared, async {
                    if outcome == "error" {
                        Err(SessionError::Storage(
                            research_agent_core::conversation::StoreError::Unavailable,
                        ))
                    } else {
                        Ok(())
                    }
                })
                .await;
            assert_eq!(result.is_ok(), outcome == "success");
        }
        assert_eq!(session.needs_recovery(), outcome != "success");
        let next =
            session.prepare_post_sampling_token_budget(turn, window, &policy(), status(), true);
        if outcome == "success" {
            let next = next.unwrap();
            assert!(next.decision().reminder.is_none() && next.decision().fallback.is_none());
        } else {
            assert!(next.is_err());
            assert!(session.request_new_context_window(turn).is_err());
        }
    }
}

#[tokio::test]
async fn prepared_claim_is_bound_to_its_writer_lease_not_only_turn_and_window() {
    let (_, _, service, first_id) = setup();
    let second_id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::from_u128(2);
    let window = Uuid::from_u128(100);
    for id in [first_id, second_id] {
        service.create(id).await.unwrap();
        service.commit(id, vec![start(id)]).await.unwrap();
        let slot = service.slot(id, false).unwrap();
        let mut owner = slot.lock().await;
        owner
            .as_mut()
            .unwrap()
            .restore_token_budget_window(turn, 0, window, None, window)
            .unwrap();
    }
    let prepared = {
        let slot = service.slot(first_id, false).unwrap();
        let owner = slot.lock().await;
        owner
            .as_ref()
            .unwrap()
            .prepare_post_sampling_token_budget(turn, window, &policy(), status(), false)
            .unwrap()
    };
    let slot = service.slot(second_id, false).unwrap();
    let mut owner = slot.lock().await;
    let session = owner.as_mut().unwrap();
    assert!(
        session
            .commit_post_sampling_token_budget(turn, prepared)
            .is_err()
    );
    let fresh = session
        .prepare_post_sampling_token_budget(turn, window, &policy(), status(), false)
        .unwrap();
    assert!(fresh.decision().reminder.is_some());
}

#[tokio::test]
async fn prepared_claim_cannot_cross_turn_ownership_and_failed_write_leaves_it_due() {
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    let first = Uuid::from_u128(100);
    let turn = Uuid::from_u128(2);
    let prepared = {
        let slot = service.slot(id, false).unwrap();
        let mut owner = slot.lock().await;
        let session = owner.as_mut().unwrap();
        session
            .restore_token_budget_window(turn, 0, first, None, first)
            .unwrap();
        let failed = session
            .prepare_post_sampling_token_budget(turn, first, &policy(), status(), false)
            .unwrap();
        assert!(failed.decision().reminder.is_some());
        drop(failed);
        session
            .prepare_post_sampling_token_budget(turn, first, &policy(), status(), false)
            .unwrap()
    };
    service
        .commit(
            id,
            vec![event(
                id,
                2,
                EventPayload::TurnFinished {
                    turn_id: turn,
                    outcome: TurnOutcome::Completed,
                },
            )],
        )
        .await
        .unwrap();
    let next = Uuid::from_u128(20);
    let mut payload = start(id).payload().clone();
    if let EventPayload::TurnStarted { turn_id, .. } = &mut payload {
        *turn_id = next;
    }
    service
        .commit(id, vec![event(id, 3, payload)])
        .await
        .unwrap();
    let slot = service.slot(id, false).unwrap();
    let mut owner = slot.lock().await;
    let session = owner.as_mut().unwrap();
    assert!(
        session
            .commit_post_sampling_token_budget(next, prepared)
            .is_err()
    );
    let fresh = session
        .prepare_post_sampling_token_budget(next, first, &policy(), status(), false)
        .unwrap();
    assert!(fresh.decision().reminder.is_some() && fresh.decision().fallback.is_some());
    let committed = session
        .commit_post_sampling_token_budget(next, fresh)
        .unwrap();
    assert!(committed.reminder.is_some() && committed.fallback.is_some());
    let once = session
        .prepare_post_sampling_token_budget(next, first, &policy(), status(), false)
        .unwrap();
    assert!(once.decision().reminder.is_none() && once.decision().fallback.is_none());
}

#[tokio::test]
async fn ended_turn_does_not_reset_claims_but_reopening_owner_does() {
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    let first = Uuid::from_u128(100);
    let template = start(id).payload().clone();
    for (offset, turn) in [
        (0, Uuid::from_u128(2)),
        (2, Uuid::from_u128(20)),
        (4, Uuid::from_u128(21)),
    ] {
        let mut payload = template.clone();
        if let EventPayload::TurnStarted { turn_id, .. } = &mut payload {
            *turn_id = turn;
        }
        service
            .commit(id, vec![event(id, offset + 1, payload)])
            .await
            .unwrap();
        {
            let slot = service.slot(id, false).unwrap();
            let mut owner = slot.lock().await;
            let session = owner.as_mut().unwrap();
            session
                .restore_token_budget_window(turn, 0, first, None, first)
                .unwrap();
            let decision = session
                .post_sampling_token_budget(turn, first, &policy(), status(), false)
                .unwrap();
            assert_eq!(decision.reminder.is_some(), offset != 2);
            assert_eq!(decision.fallback.is_some(), offset != 2);
        }
        service
            .commit(
                id,
                vec![event(
                    id,
                    offset + 2,
                    EventPayload::TurnFinished {
                        turn_id: turn,
                        outcome: TurnOutcome::Completed,
                    },
                )],
            )
            .await
            .unwrap();
        {
            let slot = service.slot(id, false).unwrap();
            let mut owner = slot.lock().await;
            assert!(
                owner
                    .as_mut()
                    .unwrap()
                    .post_sampling_token_budget(turn, first, &policy(), status(), true)
                    .is_err()
            );
        }
        if offset == 2 {
            service.release_idle(id).await.unwrap();
            service.open(id).await.unwrap();
        }
    }
}

#[tokio::test]
async fn session_window_claims_survive_repeat_preparation_and_failed_identity_changes() {
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    let slot = service.slot(id, false).unwrap();
    let mut owner = slot.lock().await;
    let session = owner.as_mut().unwrap();
    let turn = Uuid::from_u128(2);
    let first = Uuid::from_u128(100);
    let next = Uuid::from_u128(101);
    // Admission before initialization transfers to the sole window state.
    session.request_new_context_window(turn).unwrap();
    session
        .restore_token_budget_window(turn, 0, first, None, first)
        .unwrap();
    let ended = session
        .post_sampling_token_budget(turn, first, &policy(), status(), false)
        .unwrap();
    // Upstream may claim prompts at natural end, but must not consume the
    // pending reset. The next follow-up sees the request and no repeated hint.
    assert!(!ended.rollover && ended.reminder.is_some() && ended.fallback.is_some());
    let decision = session
        .post_sampling_token_budget(turn, first, &policy(), status(), true)
        .unwrap();
    assert!(decision.rollover && decision.reminder.is_none() && decision.fallback.is_none());
    session
        .restore_token_budget_window(turn, 0, first, None, first)
        .unwrap();
    let decision = session
        .post_sampling_token_budget(turn, first, &policy(), status(), true)
        .unwrap();
    assert!(!decision.rollover && decision.reminder.is_none() && decision.fallback.is_none());
    assert!(
        session
            .advance_token_budget_window(turn, next, first)
            .is_err()
    );
    assert!(
        session
            .restore_token_budget_window(turn, 1, first, Some(first), next)
            .is_err()
    );
    assert!(
        session
            .post_sampling_token_budget(Uuid::new_v4(), first, &policy(), status(), true)
            .is_err()
    );
    let decision = session
        .post_sampling_token_budget(turn, first, &policy(), status(), true)
        .unwrap();
    assert!(decision.reminder.is_none() && decision.fallback.is_none());
    session
        .advance_token_budget_window(turn, first, next)
        .unwrap();
    session
        .restore_token_budget_window(turn, 1, first, Some(first), next)
        .unwrap();
    assert!(
        session
            .post_sampling_token_budget(turn, first, &policy(), status(), true)
            .is_err()
    );
    let decision = session
        .post_sampling_token_budget(turn, next, &policy(), status(), true)
        .unwrap();
    assert!(decision.reminder.is_some() && decision.fallback.is_some());
    session.request_new_context_window(turn).unwrap();
    assert!(session.take_new_context_window_request(turn).unwrap());
    assert!(!session.take_new_context_window_request(turn).unwrap());
    session.request_new_context_window(turn).unwrap();
    session.clear_new_context_window_request();
    assert!(!session.take_new_context_window_request(turn).unwrap());
}
