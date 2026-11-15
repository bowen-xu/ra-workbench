use super::*;
use crate::agent::stop_hook_process::{StopRequest, StopTarget};
use research_agent_core::runner::StopHandlerStatus;

fn start_turn(f: &Fixture) {
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TaskKind},
    };
    let id = ConversationId::from_uuid(f.record.conversation_id);
    let event = AgentEvent::new(
        id,
        1,
        1,
        Uuid::new_v4(),
        EventPayload::TurnStarted {
            turn_id: f.record.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: f.record.context_snapshot_id,
            model: f.record.input.model.clone(),
        },
    )
    .unwrap();
    let mut db = f.store.connection().unwrap();
    crate::agent::repository::append_batch(&mut db, id, Uuid::new_v4(), 0, &[event]).unwrap();
}

fn request(f: &Fixture, target: StopTarget) -> StopRequest {
    StopRequest {
        conversation_id: f.record.conversation_id,
        turn_id: f.record.turn_id,
        model: f.record.input.model.clone(),
        permission_mode: "default".into(),
        stop_hook_active: false,
        last_assistant_message: Some("final answer".into()),
        target,
    }
}

#[tokio::test]
async fn subagent_matcher_filters_before_consent_and_freezes_target() {
    let f = Fixture::new();
    let path = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&path).unwrap();
    let entry = |id: u8, matcher: &str| {
        format!(
            "[[handlers]]\nid='00000000-0000-0000-0000-{id:012}'\nstage='subagent_stop'\nenabled=true\nmatcher='{matcher}'\nprogram='/bin/sh'\narguments=['-c','cat >/dev/null']\ntimeout_ms=5000\nmax_output_bytes=1024\n"
        )
    };
    std::fs::write(
        path.join("hooks.toml"),
        format!(
            "schema_version=1\n{}{}{}",
            entry(1, "default|worker"),
            entry(2, "other"),
            entry(3, "[")
        ),
    )
    .unwrap();
    let review = f.settings.review(f.scope()).unwrap();
    assert_eq!(review.handlers.len(), 2);
    let matched = review
        .handlers
        .iter()
        .find(|h| h.handler_id == Uuid::from_u128(1))
        .unwrap();
    f.settings
        .approve(ApproveRequest {
            scope: f.scope(),
            handler_id: matched.handler_id,
            expected_revision: matched.revision,
            fingerprint: matched.fingerprint.clone(),
        })
        .unwrap();
    let target = StopTarget::SubagentStop {
        agent_id: "child".into(),
        agent_type: "default".into(),
        agent_transcript_path: None,
    };
    let unmatched = StopTarget::SubagentStop {
        agent_id: "child".into(),
        agent_type: "unmatched".into(),
        agent_transcript_path: None,
    };
    assert!(
        f.factory
            .prepare_stop_target(AgentHost::Notes, &f.record, None, unmatched)
            .await
            .unwrap()
            .is_none()
    );
    start_turn(&f);
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop_target(AgentHost::Notes, &f.record, tools.workspace, target.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hooks.handler_ids(&target), vec![Uuid::from_u128(1)]);
    let req = request(&f, target);
    let audit = crate::agent::repository::stop_hook_audit::SqliteStopHookAudit::new(
        f.store.shared_agent_connection(),
    );
    let result = hooks
        .run_recorded(&req, &audit, std::future::pending())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.outcomes.len(), 1);
    assert_eq!(result.outcomes[0].0, Uuid::from_u128(1));
    assert!(matches!(
        result.outcomes[0].2,
        crate::agent::repository::stop_hook_audit::Outcome::Completed {
            status: crate::agent::repository::stop_hook_audit::Status::Completed,
            ..
        }
    ));
    let saved = audit
        .read(f.record.conversation_id, result.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.finished.unwrap().outcomes, result.outcomes);
    let changed = request(
        &f,
        StopTarget::SubagentStop {
            agent_id: "other".into(),
            agent_type: "default".into(),
            agent_transcript_path: None,
        },
    );
    assert!(hooks.run(&changed, &[]).await.is_err());
}

#[tokio::test]
async fn stop_ignores_even_invalid_matcher() {
    let f = Fixture::new();
    f.write(true, "stop", "cat >/dev/null");
    let path = f
        .store
        .agent_configuration_directory()
        .join("agent/hooks.toml");
    let mut config = std::fs::read_to_string(&path).unwrap();
    config.push_str("matcher='['\n");
    std::fs::write(path, config).unwrap();
    f.approve();
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop_target(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            StopTarget::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let results = hooks
        .run(
            &request(&f, StopTarget::Stop),
            &[(Uuid::from_u128(0x12), Uuid::new_v4())],
        )
        .await
        .unwrap();
    assert_eq!(results[0].output.status, StopHandlerStatus::Completed);
}

#[tokio::test]
async fn stop_config_requires_consent_and_runs_with_original_workspace() {
    let f = Fixture::new();
    f.write(
        true,
        "stop",
        "cat > stop-input.json; printf '{\"decision\":\"block\",\"reason\":\"retry\"}'",
    );
    let review = f.settings.review(f.scope()).unwrap();
    assert_eq!(review.handlers[0].stage, "stop");
    assert!(
        f.factory
            .prepare(AgentHost::Notes, &f.record, None, None)
            .await
            .unwrap()
            .is_none()
    );
    let tools = f.tools().await;
    assert!(
        f.factory
            .prepare_stop(
                AgentHost::Notes,
                &f.record,
                tools.workspace.clone(),
                research_agent_core::runner::StopEvent::Stop
            )
            .await
            .is_err()
    );
    f.approve();
    assert!(
        f.factory
            .prepare_stop(
                AgentHost::Notes,
                &f.record,
                None,
                research_agent_core::runner::StopEvent::Stop
            )
            .await
            .is_err()
    );
    assert!(
        f.factory
            .prepare_stop(
                AgentHost::Reading,
                &f.record,
                tools.workspace.clone(),
                research_agent_core::runner::StopEvent::Stop
            )
            .await
            .is_err()
    );
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace.clone(),
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let req = request(&f, StopTarget::Stop);
    let handler = Uuid::from_u128(0x12);
    let run = Uuid::new_v4();
    assert_eq!(hooks.handler_ids(&req.target), vec![handler]);
    assert!(!f.work().exists());
    assert!(hooks.run(&req, &[(handler, Uuid::nil())]).await.is_err());
    assert!(!f.work().exists());
    let mut other = req.clone();
    other.model.revision = Some("other".into());
    assert!(matches!(
        hooks.run(&other, &[(handler, run)]).await.unwrap()[0]
            .output
            .status,
        StopHandlerStatus::Failed { .. }
    ));
    assert!(!f.work().exists());
    let results = hooks.run(&req, &[(handler, run)]).await.unwrap();
    assert_eq!(results[0].run_id, run);
    assert_eq!(
        results[0].output.status,
        StopHandlerStatus::Blocked {
            feedback: "retry".into()
        }
    );
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.work().join("stop-input.json")).unwrap()).unwrap();
    assert_eq!(input["last_assistant_message"], "final answer");
}

#[tokio::test]
async fn revoked_grant_refuses_prepared_stop_without_opening_workspace() {
    let f = Fixture::new();
    f.write(true, "stop", "cat >/dev/null; touch forbidden");
    f.approve();
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace.clone(),
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let review = f.settings.review(f.scope()).unwrap().handlers.remove(0);
    f.settings
        .revoke(RevokeRequest {
            scope: f.scope(),
            handler_id: review.handler_id,
            expected_revision: review.revision,
        })
        .unwrap();
    let req = request(&f, StopTarget::Stop);
    let result = hooks
        .run(&req, &[(review.handler_id, Uuid::new_v4())])
        .await
        .unwrap();
    assert!(matches!(
        result[0].output.status,
        StopHandlerStatus::Failed { .. }
    ));
    assert!(!f.work().exists());
}

#[tokio::test]
async fn mixed_stop_handlers_are_selected_and_returned_in_declaration_order() {
    let f = Fixture::new();
    let path = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&path).unwrap();
    let entry = |id: u8, stage: &str, command: &str| {
        format!(
            "[[handlers]]\nid='00000000-0000-0000-0000-0000000000{id:02x}'\nstage='{stage}'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', {}]\ntimeout_ms=5000\nmax_output_bytes=1024\n",
            toml::Value::String(command.into())
        )
    };
    std::fs::write(
        path.join("hooks.toml"),
        format!(
            "schema_version=1\n{}{}{}",
            entry(1, "stop", "cat >/dev/null; printf first >&2; exit 2"),
            entry(
                2,
                "subagent_stop",
                "cat >/dev/null; printf child >&2; exit 2"
            ),
            entry(3, "stop", "cat >/dev/null; printf second >&2; exit 2")
        ),
    )
    .unwrap();
    for review in f
        .settings
        .review(f.scope())
        .unwrap()
        .handlers
        .into_iter()
        .filter(|r| r.stage == "stop")
    {
        f.settings
            .approve(ApproveRequest {
                scope: f.scope(),
                handler_id: review.handler_id,
                expected_revision: review.revision,
                fingerprint: review.fingerprint,
            })
            .unwrap();
    }
    let tools = f.tools().await;
    assert!(
        f.factory
            .prepare_stop(
                AgentHost::Notes,
                &f.record,
                tools.workspace.clone(),
                research_agent_core::runner::StopEvent::SubagentStop
            )
            .await
            .is_err()
    );
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace.clone(),
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    f.approve();
    let req = request(&f, StopTarget::Stop);
    let ids = hooks.handler_ids(&req.target);
    assert_eq!(ids, vec![Uuid::from_u128(1), Uuid::from_u128(3)]);
    let repeated = Uuid::new_v4();
    assert!(
        hooks
            .run(&req, &[(ids[0], repeated), (ids[1], repeated)])
            .await
            .is_err()
    );
    assert!(!f.work().exists());
    let runs = vec![(ids[0], Uuid::new_v4()), (ids[1], Uuid::new_v4())];
    let results = hooks.run(&req, &runs).await.unwrap();
    assert_eq!(
        results.iter().map(|r| r.handler_id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        results[0].output.status,
        StopHandlerStatus::Blocked {
            feedback: "first".into()
        }
    );
    assert_eq!(
        results[1].output.status,
        StopHandlerStatus::Blocked {
            feedback: "second".into()
        }
    );
    let run_ids: Vec<_> = results
        .iter()
        .map(|result| result.run_id.to_string())
        .collect();
    let aggregate = research_agent_core::runner::aggregate_stop_outputs(
        results
            .iter()
            .zip(&run_ids)
            .map(|(result, id)| (id.as_str(), &result.output)),
    );
    assert_eq!(
        aggregate
            .fragments
            .iter()
            .map(|fragment| fragment.hook_run_id.as_str())
            .collect::<Vec<_>>(),
        run_ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(
        aggregate
            .fragments
            .iter()
            .map(|fragment| fragment.text.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let sub = request(
        &f,
        StopTarget::SubagentStop {
            agent_id: "child".into(),
            agent_type: "research".into(),
            agent_transcript_path: None,
        },
    );
    assert!(hooks.handler_ids(&sub.target).is_empty());
    assert!(hooks.run(&sub, &[]).await.is_err());
    let child_hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::SubagentStop,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        child_hooks.handler_ids(&sub.target),
        vec![Uuid::from_u128(2)]
    );
    let child = child_hooks
        .run(&sub, &[(Uuid::from_u128(2), Uuid::new_v4())])
        .await
        .unwrap();
    assert_eq!(
        child[0].output.status,
        StopHandlerStatus::Blocked {
            feedback: "child".into()
        }
    );
}

#[tokio::test]
async fn absent_stop_event_does_not_require_workspace_or_other_event_consent() {
    let f = Fixture::new();
    f.write(true, "subagent_stop", "cat >/dev/null");
    assert!(
        f.factory
            .prepare_stop(
                AgentHost::Notes,
                &f.record,
                None,
                research_agent_core::runner::StopEvent::Stop
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(!f.work().exists());
}

#[tokio::test]
async fn cancellation_observes_completed_handler_and_cancels_pending_handler() {
    let f = Fixture::new();
    let path = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&path).unwrap();
    let entry = |id: u8, command: &str| {
        format!(
            "[[handlers]]\nid='00000000-0000-0000-0000-0000000000{id:02x}'\nstage='stop'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', {}]\ntimeout_ms=30000\nmax_output_bytes=1024\n",
            toml::Value::String(command.into())
        )
    };
    std::fs::write(
        path.join("hooks.toml"),
        format!(
            "schema_version=1\n{}{}",
            entry(1, "cat >/dev/null; printf first >&2; exit 2"),
            entry(
                2,
                "cat >/dev/null; printf '%s' \"$$\" > pid; exec /bin/sleep 30"
            )
        ),
    )
    .unwrap();
    f.approve();
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let req = request(&f, StopTarget::Stop);
    let runs = [
        (Uuid::from_u128(1), Uuid::new_v4()),
        (Uuid::from_u128(2), Uuid::new_v4()),
    ];
    let observer = StopHandlerCollector::new(runs.to_vec()).unwrap();
    let mut running = Box::pin(hooks.run_with_observer(&req, &runs, &observer));
    let pid = tokio::select! {
        _ = &mut running => panic!("second hook must remain active"),
        pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(f.work().join("pid")) && let Ok(pid) = value.parse::<i32>() { break pid; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }) => pid.unwrap(),
    };
    drop(running);
    let outcomes = observer.outcomes(true).unwrap();
    assert!(
        matches!(&outcomes[0].2, StopCollectedOutcome::Completed(output) if matches!(&output.status, StopHandlerStatus::Blocked { feedback } if feedback == "first"))
    );
    assert_eq!(outcomes[1].2, StopCollectedOutcome::Canceled);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                == Err(nix::errno::Errno::ESRCH)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn recorded_dispatch_commits_admission_before_running_and_result_before_return() {
    use crate::agent::repository::stop_hook_audit::{Outcome, SqliteStopHookAudit, Status};
    let f = Fixture::new();
    f.write(
        true,
        "stop",
        "cat >/dev/null; touch ran; printf retry >&2; exit 2",
    );
    f.approve();
    start_turn(&f);
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let audit = SqliteStopHookAudit::new(f.store.shared_agent_connection());
    f.store.connection().unwrap().execute_batch("create trigger deny_stop_admission before insert on agent_stop_hook_admissions begin select raise(abort,'synthetic admission failure'); end;").unwrap();
    let req = request(&f, StopTarget::Stop);
    let Err(StopRecordedError::Storage { .. }) = hooks
        .run_recorded(&req, &audit, std::future::pending())
        .await
    else {
        panic!("admission failure must report storage error");
    };
    assert!(!f.work().exists());
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger deny_stop_admission;")
        .unwrap();
    let mut wrong = req.clone();
    wrong.model.revision = Some("other".into());
    assert!(
        hooks
            .run_recorded(&wrong, &audit, std::future::pending())
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_stop_hook_admissions", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let result = hooks
        .run_recorded(&req, &audit, std::future::pending())
        .await
        .unwrap()
        .unwrap();
    assert!(!result.canceled);
    assert!(f.work().join("ran").exists());
    let StopResolution::FollowUp { parts } = result.resolution() else {
        panic!("committed feedback must request continuation");
    };
    assert_eq!(parts[0].run_id, result.outcomes[0].1);
    assert!(parts[0].input_text.contains("retry"));
    let saved = audit
        .read(f.record.conversation_id, result.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        StopResolution::from_recorded(&saved),
        Some(result.resolution())
    );
    let saved = saved.finished.unwrap();
    assert_eq!(saved.outcomes, result.outcomes);
    assert!(
        matches!(&saved.outcomes[0].2, Outcome::Completed { status: Status::Blocked { feedback }, .. } if feedback == "retry")
    );
}

#[tokio::test]
async fn committed_hook_feedback_becomes_model_input_only_after_session_journal_acceptance() {
    use crate::agent::repository::stop_hook_audit::SqliteStopHookAudit;
    use research_agent_core::{
        conversation::ConversationId,
        model::{ContentPart, ModelMessage},
        protocol::{EventPayload, ItemKind, TaskKind},
    };
    let f = Fixture::managed();
    f.write(true, "stop", "cat >/dev/null; printf retry >&2; exit 2");
    f.approve();
    let id = ConversationId::from_uuid(f.record.conversation_id);
    f.sessions.open(id).await.unwrap();
    f.sessions
        .commit_payload(
            id,
            f.record.request_id,
            EventPayload::TurnStarted {
                turn_id: f.record.turn_id,
                context_snapshot_id: f.record.context_snapshot_id,
                model: f.record.input.model.clone(),
                task_kind: TaskKind::Regular,
            },
        )
        .await
        .unwrap();
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let audit = SqliteStopHookAudit::new(f.store.shared_agent_connection());
    let result = hooks
        .run_recorded(
            &request(&f, StopTarget::Stop),
            &audit,
            std::future::pending(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result.resolution(), StopResolution::FollowUp { .. }),
        "synthetic outcomes: {:?}",
        result.outcomes
    );
    let message = super::super::stop::commit::persist_follow_up(&f.sessions, &f.record, &result)
        .await
        .unwrap();
    let ModelMessage::User {
        source_item_id: Some(item_id),
        content,
    } = message
    else {
        panic!("expected sourced hook input")
    };
    let projection = f.sessions.open(id).await.unwrap();
    let item = &projection.turn(f.record.turn_id).unwrap().items[&item_id];
    let ItemKind::HookPrompt {
        invocation_id,
        fragments,
    } = &item.kind
    else {
        panic!("expected hook item")
    };
    assert_eq!(*invocation_id, result.invocation_id);
    assert!(item.completed);
    assert!(
        content
            == fragments
                .iter()
                .map(|part| ContentPart::Text(part.input_text.clone()))
                .collect::<Vec<_>>()
    );
    let Err(super::super::stop::commit::StopPromptCommitError::Commit { invocation_id, .. }) =
        super::super::stop::commit::persist_follow_up(&f.sessions, &f.record, &result).await
    else {
        panic!("duplicate invocation must not authorize model input")
    };
    assert_eq!(invocation_id, result.invocation_id);
}

#[tokio::test]
async fn hook_feedback_cannot_use_the_legacy_unmanaged_session_store() {
    let f = Fixture::new();
    let invocation_id = Uuid::new_v4();
    let result = RecordedStopRun {
        invocation_id,
        canceled: true,
        outcomes: vec![],
    };
    assert!(
        matches!(super::super::stop::commit::persist_follow_up(&f.sessions, &f.record, &result).await,
        Err(super::super::stop::commit::StopPromptCommitError::Unmanaged(id)) if id == invocation_id)
    );
}

#[tokio::test]
async fn task_owned_stop_runtime_records_feedback_and_preserves_the_current_response() {
    use super::super::stop::runtime::{NativeStopError, NativeStopOutcome, NativeStopRuntime};
    use crate::agent::{
        cancellation::AgentCancellation, repository::stop_hook_audit::SqliteStopHookAudit,
    };
    use research_agent_core::{
        conversation::ConversationId,
        model::{
            AssistantMessage, ModelMessage, ModelOutputItem, ModelRequest, ModelResponse,
            ModelUsage,
        },
        protocol::{EventPayload, ItemKind, TaskKind},
        runner::{RunError, StopHookOutcome, StopHooks},
    };
    let f = Fixture::managed();
    f.write(
        true,
        "stop",
        "cat > stop-request.json; printf retry >&2; exit 2",
    );
    f.approve();
    let id = ConversationId::from_uuid(f.record.conversation_id);
    f.sessions.open(id).await.unwrap();
    f.sessions
        .commit_payload(
            id,
            f.record.request_id,
            EventPayload::TurnStarted {
                turn_id: f.record.turn_id,
                context_snapshot_id: f.record.context_snapshot_id,
                model: f.record.input.model.clone(),
                task_kind: TaskKind::Regular,
            },
        )
        .await
        .unwrap();
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace.clone(),
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let audit = SqliteStopHookAudit::new(f.store.shared_agent_connection());
    let signal = AgentCancellation::default();
    let runtime = NativeStopRuntime {
        handlers: &hooks,
        service: &f.sessions,
        audit: &audit,
        record: &f.record,
        signal: &signal,
        permission_mode: "default",
        target: StopTarget::Stop,
    };
    let model = f.record.input.model.clone();
    let model_request = ModelRequest {
        request_id: Uuid::new_v4(),
        model: model.clone(),
        messages: vec![],
        tools: vec![],
        max_output_tokens: 100,
        parallel_tool_calls: false,
        reasoning_effort: None,
        captured_history: None,
    };
    let response = ModelResponse {
        response_id: "synthetic".into(),
        actual_model: model,
        actual_model_observed: false,
        text: "ignored aggregate".into(),
        output: vec![ModelOutputItem::Message(AssistantMessage {
            item_id: "item".into(),
            text: "answer<oai-mem-citation>hidden</oai-mem-citation>".into(),
            phase: None,
        })],
        tool_calls: vec![],
        usage: ModelUsage {
            input_tokens: None,
            output_tokens: None,
        },
        end_turn: None,
    };
    assert!(matches!(
        runtime
            .run(Uuid::new_v4(), &model_request, &response, false)
            .await,
        Err(NativeStopError::InvalidOwner)
    ));
    let Err(RunError::EventSinkDiagnostic(detail)) =
        StopHooks::run(&runtime, Uuid::new_v4(), &model_request, &response, false).await
    else {
        panic!("expected validated stop diagnostic")
    };
    let diagnostic = serde_json::to_value(detail).unwrap();
    assert_eq!(diagnostic["code"], "stop_hook_failure");
    assert_eq!(diagnostic["safeCause"], "invalid_owner");
    assert_eq!(diagnostic["turnId"], f.record.turn_id.to_string());
    let other_audit = SqliteStopHookAudit::new(Arc::new(std::sync::Mutex::new(
        rusqlite::Connection::open_in_memory().unwrap(),
    )));
    let wrong_owner = NativeStopRuntime {
        handlers: &hooks,
        service: &f.sessions,
        audit: &other_audit,
        record: &f.record,
        signal: &signal,
        permission_mode: "default",
        target: StopTarget::Stop,
    };
    assert!(matches!(
        wrong_owner
            .run(f.record.turn_id, &model_request, &response, false)
            .await,
        Err(NativeStopError::InvalidOwner)
    ));
    let NativeStopOutcome::FollowUp(ModelMessage::User {
        source_item_id: Some(item_id),
        ..
    }) = runtime
        .run(f.record.turn_id, &model_request, &response, true)
        .await
        .unwrap()
    else {
        panic!("expected durable feedback")
    };
    let saved = f.sessions.open(id).await.unwrap();
    assert!(matches!(
        saved.turn(f.record.turn_id).unwrap().items[&item_id].kind,
        ItemKind::HookPrompt { .. }
    ));
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.work().join("stop-request.json")).unwrap())
            .unwrap();
    assert_eq!(input["last_assistant_message"], "answer");
    assert_eq!(input["stop_hook_active"], true);
    assert_eq!(input["permission_mode"], "default");
    f.write(
        true,
        "stop",
        "cat >/dev/null; printf '%s' '{\"continue\":false,\"stopReason\":\"done\"}'",
    );
    f.approve();
    let stopping = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let stopping_runtime = NativeStopRuntime {
        handlers: &stopping,
        service: &f.sessions,
        audit: &audit,
        record: &f.record,
        signal: &signal,
        permission_mode: "default",
        target: StopTarget::Stop,
    };
    match stopping_runtime
        .run(f.record.turn_id, &model_request, &response, false)
        .await
    {
        Ok(NativeStopOutcome::Stop { reason }) => assert_eq!(reason.as_deref(), Some("done")),
        Ok(NativeStopOutcome::Continue) => panic!("stop was treated as continue"),
        Ok(NativeStopOutcome::FollowUp(_)) => panic!("stop was treated as feedback"),
        Err(error) => panic!("stop dispatch failed: {error:?}"),
    }
    assert!(matches!(
        StopHooks::run(
            &stopping_runtime,
            f.record.turn_id,
            &model_request,
            &response,
            false
        )
        .await,
        Ok(StopHookOutcome::Stop)
    ));
    signal.cancel();
    assert!(matches!(
        runtime
            .run(f.record.turn_id, &model_request, &response, false)
            .await,
        Err(NativeStopError::Canceled)
    ));
    assert!(matches!(
        StopHooks::run(&runtime, f.record.turn_id, &model_request, &response, false).await,
        Err(RunError::Model(
            research_agent_core::model::ModelError::Canceled
        ))
    ));
}

#[tokio::test]
async fn failed_finish_keeps_admission_incomplete_and_never_returns_feedback() {
    use crate::agent::repository::stop_hook_audit::SqliteStopHookAudit;
    let f = Fixture::new();
    f.write(
        true,
        "stop",
        "cat >/dev/null; touch ran; printf retry >&2; exit 2",
    );
    f.approve();
    start_turn(&f);
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let audit = SqliteStopHookAudit::new(f.store.shared_agent_connection());
    f.store.connection().unwrap().execute_batch("create trigger deny_stop_finish before insert on agent_stop_hook_finishes begin select raise(abort,'synthetic finish failure'); end;").unwrap();
    let req = request(&f, StopTarget::Stop);
    let Err(StopRecordedError::Storage {
        invocation_id: failed_id,
        ..
    }) = hooks
        .run_recorded(&req, &audit, std::future::pending())
        .await
    else {
        panic!("finish failure must retain invocation identity");
    };
    assert!(f.work().join("ran").exists());
    let db = f.store.connection().unwrap();
    let invocation: String = db
        .query_row(
            "select invocation_id from agent_stop_hook_admissions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(invocation, failed_id.to_string());
    assert_eq!(
        db.query_row("select count(*) from agent_stop_hook_results", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(db);
    let incomplete = audit
        .read(
            f.record.conversation_id,
            Uuid::parse_str(&invocation).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(incomplete.finished.is_none());
    assert!(StopResolution::from_recorded(&incomplete).is_none());
}

#[tokio::test]
async fn recorded_cancellation_persists_completed_and_interrupted_handlers() {
    use crate::agent::repository::stop_hook_audit::{Outcome, SqliteStopHookAudit, Status};
    let f = Fixture::new();
    let path = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&path).unwrap();
    let entry = |id: u8, command: &str| {
        format!(
            "[[handlers]]\nid='00000000-0000-0000-0000-0000000000{id:02x}'\nstage='stop'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', {}]\ntimeout_ms=30000\nmax_output_bytes=1024\n",
            toml::Value::String(command.into())
        )
    };
    std::fs::write(
        path.join("hooks.toml"),
        format!(
            "schema_version=1\n{}{}",
            entry(1, "cat >/dev/null; printf first >&2; exit 2"),
            entry(2, "cat >/dev/null; touch running; exec /bin/sleep 30")
        ),
    )
    .unwrap();
    f.approve();
    start_turn(&f);
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare_stop(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            research_agent_core::runner::StopEvent::Stop,
        )
        .await
        .unwrap()
        .unwrap();
    let audit = SqliteStopHookAudit::new(f.store.shared_agent_connection());
    let req = request(&f, StopTarget::Stop);
    let (cancel, canceled) = tokio::sync::oneshot::channel::<()>();
    let mut run = Box::pin(hooks.run_recorded(&req, &audit, async {
        let _ = canceled.await;
    }));
    tokio::select! {
        _ = &mut run => panic!("second handler must remain active"),
        ready = tokio::time::timeout(Duration::from_secs(5), async {
            while !f.work().join("running").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }) => ready.unwrap(),
    }
    cancel.send(()).unwrap();
    let result = run.await.unwrap().unwrap();
    assert!(result.canceled);
    let saved = audit
        .read(f.record.conversation_id, result.invocation_id)
        .await
        .unwrap()
        .unwrap()
        .finished
        .unwrap();
    assert_eq!(saved.outcomes, result.outcomes);
    assert!(
        matches!(&saved.outcomes[0].2, Outcome::Completed { status: Status::Blocked { feedback }, .. } if feedback == "first")
    );
    assert_eq!(saved.outcomes[1].2, Outcome::Canceled);
}
