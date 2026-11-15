use super::*;
use crate::agent::repository;
use research_agent_core::{
    context::{checkpoint::ContextCheckpoint, compaction_strategy::*},
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind},
};

fn window(f: &Fixture) -> CompactionHookRequest {
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    let mut db = f.store.connection().unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.record.conversation_id),
        1,
        1,
        f.record.request_id,
        EventPayload::TurnStarted {
            turn_id: f.record.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: f.record.context_snapshot_id,
            model: f.record.input.model.clone(),
        },
    )
    .unwrap();
    repository::append_batch(
        &mut db,
        event.conversation_id(),
        f.record.request_id,
        0,
        &[event],
    )
    .unwrap();
    let checkpoint = ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: f.record.conversation_id,
        turn_id: f.record.turn_id,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model: f.record.input.model.clone(),
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("summary".into()),
        retained: vec![],
        canonical_context: None,
    };
    repository::context_windows::install(&mut db, &checkpoint).unwrap();
    let mut request = f.request(CompactionHookStage::SessionStartCompact);
    request.operation_id = checkpoint.window_id;
    repository::compaction_hook_audit::record(
        &db,
        &request,
        CompactionHookNotification::Started,
        1,
    )
    .unwrap();
    request
}
#[tokio::test]
async fn native_factory_requires_frozen_budget_and_exact_model_before_commands_can_run() {
    let fixture = Fixture::new();
    fixture.write(true, "pre_compact", "cat >/dev/null; touch forbidden");
    fixture.approve();
    let tools = fixture.tools().await;
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                tools.workspace.clone(),
                None
            )
            .await
            .is_err()
    );
    let mut other = fixture.record.input.model.clone();
    other.revision = Some("foreign".into());
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                tools.workspace,
                Some(HookContextBudget::new(other, 512).unwrap())
            )
            .await
            .is_err()
    );
    assert!(!fixture.work().exists());
}
#[tokio::test]
async fn native_factory_freezes_preview_budget_and_persists_full_output_before_context() {
    let fixture = Fixture::new();
    fixture.write(
        true,
        "session_start_compact",
        "cat >/dev/null; printf '%0900d' 0",
    );
    fixture.approve();
    let request = window(&fixture);
    let tools = fixture.tools().await;
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace.clone(),
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    // A different future profile does not change this already-prepared task.
    let future_hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 1000).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let output = hooks.run_with_context(request.clone()).await;
    assert_eq!(output.outcome, CompactionHookOutcome::Continue);
    assert_eq!(output.additional_contexts[0].len(), 900);
    let prepared = hooks
        .prepare_context(request.clone(), output)
        .await
        .unwrap();
    assert!(prepared.additional_contexts[0].len() <= 512);
    assert!(prepared.additional_contexts[0].contains("hook-artifact://"));
    assert!(prepared.additional_contexts[0].contains("truncated"));
    let full = future_hooks.run_with_context(request.clone()).await;
    let full = future_hooks
        .prepare_context(request.clone(), full)
        .await
        .unwrap();
    assert_eq!(full.additional_contexts[0].len(), 900);
    assert!(!full.additional_contexts[0].contains("hook-artifact://"));
    {
        let mut db = fixture.store.connection().unwrap();
        assert_eq!(db.query_row("select byte_size from content_objects where sha256=(select content_sha256 from agent_hook_spill_artifacts)",[],|r|r.get::<_,i64>(0)).unwrap(),900);
        assert_eq!(
            db.query_row("select count(*) from agent_hook_spill_grants", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        repository::compaction_hook_audit::context::record_result(&mut db, &request, &prepared, 2)
            .unwrap();
        assert_eq!(
            db.query_row("select count(*) from agent_hook_spill_grants", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    fixture.sessions.processes().shutdown().await.unwrap();
}
