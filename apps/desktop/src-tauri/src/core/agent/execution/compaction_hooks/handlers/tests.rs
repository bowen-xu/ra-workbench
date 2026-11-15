use super::*;

fn collector() -> HandlerCollector {
    HandlerCollector::new(
        CompactionHookRequest {
            operation_id: Uuid::from_u128(1),
            conversation_id: Uuid::from_u128(2),
            turn_id: Uuid::from_u128(3),
            phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
            stage: CompactionHookStage::PreCompact,
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "test".into(),
                model_id: "model".into(),
                revision: None,
            },
        },
        vec![Uuid::from_u128(10), Uuid::from_u128(11)],
    )
    .unwrap()
}

#[test]
fn missing_completion_rejects_success_but_cancellation_preserves_finished_status() {
    let collector = collector();
    collector.notify(
        Uuid::from_u128(10),
        &collector.request,
        CompactionHookNotification::Started,
    );
    collector.notify(
        Uuid::from_u128(10),
        &collector.request,
        CompactionHookNotification::Completed(CompactionHookOutcome::Continue),
    );
    assert!(
        collector
            .completed(CompactionHookOutcome::Continue)
            .is_none()
    );
    assert_eq!(
        collector
            .completed(CompactionHookOutcome::Canceled)
            .unwrap(),
        [
            (Uuid::from_u128(10), Some(CompactionHookOutcome::Continue)),
            (Uuid::from_u128(11), Some(CompactionHookOutcome::Canceled)),
        ]
    );
}

#[test]
fn duplicate_unknown_or_wrong_identity_callbacks_cannot_complete_audit() {
    for mode in 0..3 {
        let collector = collector();
        let mut request = collector.request.clone();
        let mut id = Uuid::from_u128(10);
        match mode {
            0 => collector.notify(id, &request, CompactionHookNotification::Started),
            1 => id = Uuid::from_u128(99),
            _ => request.turn_id = Uuid::new_v4(),
        }
        collector.notify(id, &request, CompactionHookNotification::Started);
        assert!(
            collector
                .completed(CompactionHookOutcome::Canceled)
                .is_none()
        );
    }
}
