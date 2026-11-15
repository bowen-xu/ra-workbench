use super::*;
use crate::agent::{
    execution_profile::AgentExecutionMode,
    note_start::{self, StartRequest},
    repository::SqliteConversationStore,
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use research_agent_core::{model::*, protocol::TurnOutcome};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) async fn saved(f: &Fixture) -> SubmissionRecord {
    let mut input = request(f, Uuid::new_v4());
    input.model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    prepare(&f.store, &f.models, input).await.unwrap()
}
pub(super) fn confirmation(f: &Fixture, record: &SubmissionRecord) -> StartRequest {
    StartRequest {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        page_id: f.page,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: AgentExecutionMode::DeepSeekNonThinking,
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn note_task_runs_authorized_stop_on_its_own_workspace() {
    run_note_stop(false).await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn child_note_runs_subagent_stop_with_default_role_and_child_identity() {
    run_note_stop(true).await;
}

#[cfg(target_os = "macos")]
async fn run_note_stop(child: bool) {
    use super::runner::Transport;
    use crate::agent::compaction_hook_commands::{ApproveRequest, HookConsentService, Host, Scope};
    use std::{collections::VecDeque, sync::Mutex};
    let f = fixture_with_owner_at(
        true,
        true,
        tempfile::Builder::new()
            .prefix("note-stop-")
            .tempdir_in("/private/tmp")
            .unwrap(),
    );
    f.store.connection().unwrap().execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values(?1,'note',?2,'r1')", rusqlite::params![f.conversation.to_string(), f.page.to_string()]).unwrap();
    let record = saved(&f).await;
    if child {
        f.store.connection().unwrap().execute("insert into agent_subtasks(child_conversation_id,parent_conversation_id,parent_context_snapshot_id,child_context_snapshot_id,depth,model_calls,tool_calls,status) values(?1,?2,?3,?4,1,1,1,'running')", rusqlite::params![f.conversation.to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string(),record.context_snapshot_id.to_string()]).unwrap();
    }
    crate::agent::host_execution::freeze_grant_sql(&f.store.connection().unwrap(), &record)
        .unwrap();
    let config = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config).unwrap();
    let command = if child {
        format!(
            "input=$(cat); case \"$input\" in *'\"agent_id\":\"{}\"'*'\"agent_type\":\"default\"'*) printf '{{\"continue\":false}}' ;; *) printf 'incorrect child identity' >&2; exit 2 ;; esac",
            f.conversation
        )
    } else {
        "cat >/dev/null; printf '{\"continue\":false}'".into()
    };
    let stage = if child { "subagent_stop" } else { "stop" };
    let command = toml::Value::String(command);
    std::fs::write(config.join("hooks.toml"), format!("schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='{stage}'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', {command}]\ntimeout_ms=5000\nmax_output_bytes=1024\n")).unwrap();
    let settings = Arc::new(HookConsentService::from_app_store(&f.store));
    let scope = Scope {
        host: Host::Notes,
        model: record.input.model.clone(),
    };
    let review = settings.review(scope.clone()).unwrap().handlers.remove(0);
    settings
        .approve(ApproveRequest {
            scope,
            handler_id: review.handler_id,
            expected_revision: review.revision,
            fingerprint: review.fingerprint,
        })
        .unwrap();
    let models = f.models.clone().with_native_compaction_hooks(settings);
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([vec![
            ModelStreamEvent::Started {
                response_id: "note-stop".into(),
                actual_model: record.input.model.clone(),
            },
            ModelStreamEvent::TextDelta("Answer".into()),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                end_turn: None,
            },
        ]])),
    });
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    let admitted = note_start::start_selected_with_tools_factory(
        &f.store,
        models,
        sessions,
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("new run")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let db = f.store.connection().unwrap();
    let count: i64 = db
        .query_row("select count(*) from agent_stop_hook_finishes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    let event: String = db
        .query_row(
            "select event_name from agent_stop_hook_admissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event, if child { "subagent_stop" } else { "stop" });
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn child_task_does_not_prepare_parent_stop_hooks() {
    use crate::agent::compaction_hook_commands::HookConsentService;
    let f = fixture_with_owner_at(
        true,
        true,
        tempfile::Builder::new()
            .prefix("note-child-stop-")
            .tempdir_in("/private/tmp")
            .unwrap(),
    );
    let record = saved(&f).await;
    f.store.connection().unwrap().execute("insert into agent_subtasks(child_conversation_id,parent_conversation_id,parent_context_snapshot_id,child_context_snapshot_id,depth,model_calls,tool_calls,status) values(?1,?2,?3,?4,1,1,1,'running')", rusqlite::params![f.conversation.to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string(),record.context_snapshot_id.to_string()]).unwrap();
    let config = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("hooks.toml"), "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='stop'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', 'exit 1']\ntimeout_ms=5000\nmax_output_bytes=1024\n").unwrap();
    let models = f
        .models
        .clone()
        .with_native_compaction_hooks(Arc::new(HookConsentService::from_app_store(&f.store)));
    assert!(
        crate::agent::execution::NativeStopBinding::prepare(
            &models,
            crate::agent::model::AgentHost::Notes,
            &f.store,
            &record,
            None
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn unknown_page_note_child_creation_stops_model_continuation() {
    use super::runner::Transport;
    use std::{collections::VecDeque, sync::Mutex};
    let f = fixture_with_owner(true, true);
    let record = saved(&f).await;
    f.store.connection().unwrap().execute_batch("create table child_unknown_parent(id integer primary key);
      create table child_unknown_deferred(id integer references child_unknown_parent(id) deferrable initially deferred);
      create trigger child_unknown_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound'
      begin insert into child_unknown_deferred(id) values(9); end;").unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([vec![
            ModelStreamEvent::Started {
                response_id: "child-fixture".into(),
                actual_model: record.input.model.clone(),
            },
            ModelStreamEvent::ToolCallStarted {
                call_id: "child-call".into(),
                name: "research_spawn_subtask".into(),
            },
            ModelStreamEvent::ToolArgumentsDelta {
                call_id: "child-call".into(),
                delta: r#"{"prompt":"Compare approaches"}"#.into(),
            },
            ModelStreamEvent::ToolCallCompleted {
                call_id: "child-call".into(),
            },
            ModelStreamEvent::Completed {
                reason: FinishReason::ToolCalls,
                end_turn: None,
            },
        ]])),
    });
    let admission = note_start::start_selected_with_tools_factory(
        &f.store,
        f.models.clone(),
        sessions(&f),
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new run")
    };
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(15), owned.task.wait())
            .await
            .expect("child creation must settle without deadlock"),
        TaskCompletion::RecoveryRequired
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert!(
        f.store.connection().is_err(),
        "the child commit fault must block its original owner"
    );
    let Fixture {
        root,
        store,
        models,
        conversation,
        page,
    } = f;
    drop(owned);
    drop(transport);
    drop(models);
    drop(store);
    let sql = rusqlite::Connection::open(root.path().join("fixture.sqlite")).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let reopened = AppStore::for_test_database(db, root.path().join("data/objects"));
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            reopened.shared_agent_connection(),
            "page-child-reopen".into(),
        )
        .unwrap(),
    )));
    let history = service
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    use research_agent_core::submission::SubmissionStore;
    let restored = crate::agent::repository::SqliteSubmissionStore::for_app(&reopened)
        .get_by_turn(conversation, record.turn_id)
        .await
        .unwrap();
    assert_eq!(
        restored, record,
        "the frozen parent request survives reopening"
    );
    let binding: (String, String) = reopened.connection().unwrap().query_row(
        "select resource_kind,resource_id from agent_conversation_bindings where conversation_id=?1 and resource_kind='note'",
        [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(binding, ("note".into(), page.to_string()));
    assert!(history.has_unknown_tool_outcome());
    let turn = history.turn(record.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
    let call_id = turn
        .items
        .values()
        .find_map(|item| match &item.kind {
            research_agent_core::protocol::ItemKind::ToolCall {
                call_id,
                name,
                provider_call_id: Some(provider),
                ..
            } if item.accepted
                && item.completed
                && name == "research_spawn_subtask"
                && provider == "child-call" =>
            {
                Some(*call_id)
            }
            _ => None,
        })
        .expect("original accepted child call");
    assert!(!turn.items.values().any(|item| item.completed
        && matches!(item.kind,
        research_agent_core::protocol::ItemKind::ToolResult { call_id: id, .. } if id == call_id)));
    let creation = crate::agent::child_creation::intent(
        crate::agent::child_creation::note_tool_child_id(record.turn_id, "child-call"),
        research_agent_core::conversation::ConversationBinding {
            kind: research_agent_core::conversation::BindingKind::Note,
            resource_id: page.to_string(),
        },
        crate::agent::child_creation::Host::Note,
    );
    assert_eq!(turn.child_creation_intents.get(&call_id), Some(&creation));
    assert_eq!(
        turn.child_submission_requests.get(&call_id),
        Some(
            &research_agent_core::conversation::child_submission_request_id(*creation.id.as_uuid())
        )
    );
    use research_agent_core::conversation::{ConversationCatalog, CreateBoundStatus};
    let catalog = crate::agent::repository::SqliteConversationCatalog::from_app_store(&reopened);
    let request = || crate::agent::child_creation::status::Request {
        schema_version: 1,
        conversation_id: conversation,
        turn_id: record.turn_id,
        call_id,
    };
    let response = crate::agent::child_creation::status::execute(&service, &catalog, request())
        .await
        .unwrap();
    let wire = serde_json::to_value(response).unwrap();
    assert_eq!(wire["conversationId"], conversation.to_string());
    assert_eq!(wire["callId"], call_id.to_string());
    assert_eq!(
        wire["creation"]["conversationId"],
        creation.id.as_uuid().to_string()
    );
    assert_eq!(
        wire["creation"]["operationId"],
        creation.operation_id.to_string()
    );
    assert_eq!(wire["creation"]["status"]["type"], "noSqlReceipt");
    let mut missing = request();
    missing.call_id = Uuid::new_v4();
    let missing_error = crate::agent::child_creation::status::execute(&service, &catalog, missing)
        .await
        .unwrap_err();
    assert!(matches!(
        &missing_error.kind,
        crate::agent::child_creation::status::ErrorKind::IntentUnavailable
    ));
    let missing_wire = serde_json::to_value(missing_error).unwrap();
    assert_eq!(missing_wire["code"], "intentUnavailable");
    assert_eq!(missing_wire["turnId"], record.turn_id.to_string());
    assert!(matches!(
        catalog.create_bound_status(creation).await.unwrap(),
        CreateBoundStatus::NoSqlReceipt { closed: false, .. }
    ));
    assert!(matches!(
        service
            .rollback_conversation(crate::agent::service::ConversationRollbackRequest {
                conversation_id: ConversationId::from_uuid(conversation),
                request_id: Uuid::new_v4(),
                expected_watermark: history.seq(),
                expected_last_turn: record.turn_id,
                user_turns: 1,
            })
            .await,
        Err(crate::agent::service::AgentServiceError::Session(
            research_agent_core::session::SessionError::RecoveryRequired
        ))
    ));
}
pub(super) fn sessions(f: &Fixture) -> Arc<AgentSessionService> {
    Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            f.store.shared_agent_connection(),
            "note-start-fixture".into(),
        )
        .unwrap(),
    )))
}
pub(super) fn no_items(f: &Fixture) {
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn mismatched_confirmation_and_unsupported_profile_never_prepare_transport() {
    let f = fixture(true);
    let record = saved(&f).await;
    let service = sessions(&f);
    for case in 0..4 {
        let mut request = confirmation(&f, &record);
        match case {
            0 => request.context_snapshot_id = Uuid::new_v4(),
            1 => request.model.model_id.push_str("-other"),
            2 => request.page_id = Uuid::new_v4(),
            _ => request.mode = AgentExecutionMode::ChatGptDefault,
        }
        let result =
            note_start::start_with_factory(&f.store, service.clone(), request, |_, _| async {
                panic!("invalid admission reached credentials")
            })
            .await;
        match case {
            0 | 1 => assert!(matches!(
                result,
                Err(note_start::Error::ConfirmationMismatch)
            )),
            2 => assert!(matches!(
                result,
                Err(note_start::Error::Preparation(Error::Unbound))
            )),
            _ => assert!(matches!(result, Err(note_start::Error::UnsupportedModel))),
        }
    }
    no_items(&f);
}

pub(super) struct PendingTransport {
    pub(super) calls: AtomicUsize,
    pub(super) reached: tokio::sync::Notify,
}
impl ModelTransport for PendingTransport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.reached.notify_one();
            std::future::pending().await
        })
    }
}
pub(super) fn pending_transport() -> Arc<PendingTransport> {
    Arc::new(PendingTransport {
        calls: AtomicUsize::new(0),
        reached: tokio::sync::Notify::new(),
    })
}

#[tokio::test]
async fn binding_change_while_preparing_transport_rejects_before_durable_admission() {
    let f = fixture(true);
    let record = saved(&f).await;
    let transport = pending_transport();
    let result = note_start::start_with_factory(
        &f.store,
        sessions(&f),
        confirmation(&f, &record),
        |_, _| async {
            f.store
                .connection()
                .unwrap()
                .execute("delete from agent_conversation_bindings", [])
                .unwrap();
            Ok(transport.clone() as Arc<dyn ModelTransport>)
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(note_start::Error::Preparation(Error::Unbound))
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    no_items(&f);
}

#[tokio::test]
async fn shared_stop_cancels_note_and_retries_do_not_refresh_disabled_model() {
    let f = fixture(true);
    let record = saved(&f).await;
    let service = sessions(&f);
    let transport = pending_transport();
    let first = note_start::start_with_factory(
        &f.store,
        service.clone(),
        confirmation(&f, &record),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = first else {
        panic!("expected new turn")
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.reached.notified(),
    )
    .await
    .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='deepseek'",
            [],
        )
        .unwrap();
    assert!(matches!(
        note_start::start(
            &f.store,
            &f.models,
            service.clone(),
            confirmation(&f, &record)
        )
        .await
        .unwrap(),
        SubmissionAdmission::AlreadyStarted { outcome: None, .. }
    ));
    let id = ConversationId::from_uuid(f.conversation);
    assert!(service.tasks().cancel_turn(id, owned.turn_id).unwrap());
    assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
    assert!(matches!(
        note_start::start(
            &f.store,
            &f.models,
            service.clone(),
            confirmation(&f, &record)
        )
        .await
        .unwrap(),
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Canceled),
            ..
        }
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        service
            .open(id)
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Canceled)
    );
}

#[tokio::test]
async fn missing_model_configuration_fails_without_starting_a_note_turn() {
    let f = fixture(true);
    let record = saved(&f).await;
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='deepseek'",
            [],
        )
        .unwrap();
    assert!(matches!(
        note_start::start(&f.store, &f.models, sessions(&f), confirmation(&f, &record)).await,
        Err(note_start::Error::ModelUnavailable)
    ));
    no_items(&f);
}

#[test]
fn start_wire_rejects_renderer_execution_overrides() {
    let base = serde_json::json!({"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"contextSnapshotId":Uuid::new_v4(),"model":{"providerId":"deepseek","modelId":"deepseek-v4-pro","revision":null},"mode":"deepSeekNonThinking"});
    assert!(serde_json::from_value::<StartRequest>(base.clone()).is_ok());
    for key in [
        "tools",
        "text",
        "systemInstructions",
        "maxOutputTokens",
        "credential",
    ] {
        let mut invalid = base.clone();
        invalid[key] = serde_json::json!("override");
        assert!(serde_json::from_value::<StartRequest>(invalid).is_err());
    }
}

struct Events(std::collections::VecDeque<ModelStreamEvent>);
impl ModelStream for Events {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
pub(super) struct RevokeTransport {
    pub(super) store: AppStore,
    pub(super) source: String,
    pub(super) calls: AtomicUsize,
}
impl ModelTransport for RevokeTransport {
    fn capabilities(&self) -> ModelCapabilities {
        pending_transport().capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.store
                .connection()
                .unwrap()
                .execute("delete from agent_conversation_bindings", [])
                .unwrap();
            Ok(Box::new(Events(std::collections::VecDeque::from([
                ModelStreamEvent::Started {
                    response_id: "revoke-fixture".into(),
                    actual_model: request.model,
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "read".into(),
                    name: "research_read_source".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "read".into(),
                    delta: serde_json::json!({"sourceId":self.source}).to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "read".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ]))) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn binding_removal_during_request_blocks_subsequent_model_disclosure() {
    let f = fixture(true);
    let record = saved(&f).await;
    let service = sessions(&f);
    let transport = Arc::new(RevokeTransport {
        store: f.store.clone(),
        source: record.input.context[0].source_id.clone(),
        calls: AtomicUsize::new(0),
    });
    let admission = note_start::start_with_factory(
        &f.store,
        service.clone(),
        confirmation(&f, &record),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("expected new turn")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Failed);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        service
            .open(ConversationId::from_uuid(f.conversation))
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Failed)
    );
}
