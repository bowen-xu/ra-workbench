use super::super::{
    runner::Transport,
    start::{RevokeTransport, no_items, pending_transport, sessions},
};
use super::*;
use crate::agent::{
    execution_profile::AgentExecutionMode, note_start, reading_note_start,
    service::SubmissionAdmission, tasks::TaskCompletion,
};
use research_agent_core::{model::*, protocol::TurnOutcome};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

async fn saved(f: &Fixture) -> SubmissionRecord {
    let mut request = reading_request(f, Uuid::new_v4());
    request.model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    reading::prepare(&f.store, &f.models, request)
        .await
        .unwrap()
}
fn confirmation(f: &Fixture, record: &SubmissionRecord) -> reading_note_start::StartRequest {
    reading_note_start::StartRequest {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        reading_note_id: f.page,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: AgentExecutionMode::DeepSeekNonThinking,
    }
}
#[tokio::test]
async fn invalid_reading_confirmation_and_revoked_binding_never_admit_a_turn() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let service = sessions(&f);
    for case in 0..4 {
        let mut request = confirmation(&f, &record);
        match case {
            0 => request.context_snapshot_id = Uuid::new_v4(),
            1 => request.model.model_id.push_str("-other"),
            2 => request.reading_note_id = Uuid::new_v4(),
            _ => request.mode = AgentExecutionMode::ChatGptDefault,
        }
        assert!(
            note_start::start_selected_with_factory(
                &f.store,
                service.clone(),
                request.into(),
                |_, _| async { panic!("invalid confirmation reached transport") }
            )
            .await
            .is_err()
        );
    }
    let transport = pending_transport();
    let result = note_start::start_selected_with_factory(
        &f.store,
        service,
        confirmation(&f, &record).into(),
        |_, _| async {
            f.store
                .connection()
                .unwrap()
                .execute(
                    "delete from agent_conversation_bindings where resource_kind='readingNote'",
                    [],
                )
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
async fn reading_stop_and_duplicate_start_share_durable_task_state_without_refreshing_auth() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let service = sessions(&f);
    let transport = pending_transport();
    let admission = note_start::start_selected_with_factory(
        &f.store,
        service.clone(),
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new run")
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
        .execute("update llm_provider_configs set enabled=0", [])
        .unwrap();
    assert!(matches!(
        reading_note_start::start(
            &f.store,
            &f.models,
            service.clone(),
            confirmation(&f, &record)
        )
        .await
        .unwrap(),
        SubmissionAdmission::AlreadyStarted { outcome: None, .. }
    ));
    assert!(
        service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(f.conversation), owned.turn_id)
            .unwrap()
    );
    assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
    assert!(matches!(
        reading_note_start::start(&f.store, &f.models, service, confirmation(&f, &record))
            .await
            .unwrap(),
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Canceled),
            ..
        }
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn reading_binding_revocation_during_model_call_blocks_next_disclosure() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let service = sessions(&f);
    let transport = Arc::new(RevokeTransport {
        store: f.store.clone(),
        source: record.input.context[0].source_id.clone(),
        calls: AtomicUsize::new(0),
    });
    let admission = note_start::start_selected_with_factory(
        &f.store,
        service.clone(),
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new run")
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
#[tokio::test]
async fn reading_source_loop_keeps_frozen_content_and_completed_run_is_not_replayed() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let service = sessions(&f);
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([
            vec![
                ModelStreamEvent::Started {
                    response_id: "first".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "read".into(),
                    name: "research_read_source".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "read".into(),
                    delta: serde_json::json!({"sourceId":record.input.context[0].source_id})
                        .to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "read".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ],
            vec![
                ModelStreamEvent::Started {
                    response_id: "second".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::TextDelta("A source-grounded explanation".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ],
        ])),
    });
    f.store
        .connection()
        .unwrap()
        .execute(
            "update reading_note_sources set quote_exact='Changed live quote'",
            [],
        )
        .unwrap();
    let admission = note_start::start_selected_with_factory(
        &f.store,
        service.clone(),
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new run")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert!(matches!(
        note_start::start_selected_with_factory(
            &f.store,
            service,
            confirmation(&f, &record).into(),
            |_, _| async { panic!("completed replay reached transport") }
        )
        .await
        .unwrap(),
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|t| t.name == "research_read_source")
    );
    assert!(
        !requests[0]
            .tools
            .iter()
            .any(|t| t.name == "research_propose_note_rewrite")
    );
    let output = requests[1]
        .messages
        .iter()
        .find_map(|m| match m {
            ModelMessage::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    assert!(
        output.contains("Retained source quote")
            && output.contains("My reading interpretation")
            && !output.contains("Changed live quote")
    );
    assert!(output.contains(&record.context_snapshot_id.to_string()));
}

#[tokio::test]
async fn reading_note_registers_the_shared_bounded_child_tool() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([vec![
            ModelStreamEvent::Started {
                response_id: "answer".into(),
                actual_model: record.input.model.clone(),
            },
            ModelStreamEvent::TextDelta("No delegation needed.".into()),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
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
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "research_spawn_subtask")
    );
    assert!(
        !requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "research_propose_note_rewrite")
    );
}

#[tokio::test]
async fn unknown_reading_note_child_creation_stops_model_continuation() {
    let f = managed_reading_fixture();
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
    // Drop the blocked owner, then reopen the same SQLite/Raft pair. The
    // accepted call and parent scope must remain usable without resampling.
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
    let service = Arc::new(crate::agent::service::AgentSessionService::new(Arc::new(
        crate::agent::repository::SqliteConversationStore::new(
            reopened.shared_agent_connection(),
            "child-reopen".into(),
        )
        .unwrap(),
    )));
    let history = service
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    assert!(history.has_unknown_tool_outcome());
    let rollback = service
        .rollback_conversation(crate::agent::service::ConversationRollbackRequest {
            conversation_id: ConversationId::from_uuid(conversation),
            request_id: Uuid::new_v4(),
            expected_watermark: history.seq(),
            expected_last_turn: record.turn_id,
            user_turns: 1,
        })
        .await;
    assert!(matches!(
        rollback,
        Err(crate::agent::service::AgentServiceError::Session(
            research_agent_core::session::SessionError::RecoveryRequired
        ))
    ));
    let turn = history.turn(record.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
    assert!(!turn.items.values().any(|item| item.completed
        && matches!(
            item.kind,
            research_agent_core::protocol::ItemKind::ToolResult { .. }
        )));
    let (call_id, provider_call) = turn
        .items
        .values()
        .find_map(|item| match &item.kind {
            research_agent_core::protocol::ItemKind::ToolCall {
                name,
                call_id,
                provider_call_id: Some(id),
                ..
            } if item.completed && name == "research_spawn_subtask" => {
                Some((*call_id, id.as_str()))
            }
            _ => None,
        })
        .expect("original accepted child call");
    assert_eq!(provider_call, "child-call");
    use research_agent_core::submission::SubmissionStore;
    let restored = crate::agent::repository::SqliteSubmissionStore::for_app(&reopened)
        .get_by_turn(conversation, record.turn_id)
        .await
        .unwrap();
    assert_eq!(restored, record);
    let source = restored
        .input
        .context
        .iter()
        .find(|source| source.source_id == format!("reading-note:{page}"))
        .expect("original frozen reading note scope");
    let anchor: serde_json::Value =
        serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
    assert_eq!(anchor["readingNoteId"], page.to_string());
    let binding = {
        let db = reopened.connection().unwrap();
        let (kind, resource): (String, String) = db.query_row(
            "select resource_kind,resource_id from agent_conversation_bindings where conversation_id=?1 and resource_kind='readingNote'",
            [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(kind, "readingNote");
        assert_eq!(resource, page.to_string());
        research_agent_core::conversation::ConversationBinding {
            kind: research_agent_core::conversation::BindingKind::ReadingNote,
            resource_id: resource,
        }
    };
    let child = crate::agent::child_creation::note_tool_child_id(record.turn_id, provider_call);
    let creation = crate::agent::child_creation::intent(
        child,
        binding,
        crate::agent::child_creation::Host::Note,
    );
    assert_eq!(turn.child_creation_intents.get(&call_id), Some(&creation));
    assert_eq!(
        turn.child_submission_requests.get(&call_id),
        Some(&research_agent_core::conversation::child_submission_request_id(child))
    );
    use research_agent_core::conversation::{ConversationCatalog, CreateBoundStatus};
    let catalog = crate::agent::repository::SqliteConversationCatalog::from_app_store(&reopened);
    assert!(matches!(
        catalog.create_bound_status(creation).await.unwrap(),
        CreateBoundStatus::NoSqlReceipt { closed: false, .. }
    ));
}

#[tokio::test]
async fn reading_run_rejects_page_rewrite_calls_without_changing_either_entity() {
    let f = reading_fixture();
    let record = saved(&f).await;
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([
            vec![
                ModelStreamEvent::Started {
                    response_id: "unexpected-tool".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "rewrite".into(),
                    name: "research_propose_note_rewrite".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "rewrite".into(),
                    delta:
                        serde_json::json!({"sourceId":record.input.context[0].source_id,"edits":[]})
                            .to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "rewrite".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ],
            vec![
                ModelStreamEvent::Started {
                    response_id: "after-rejected-tool".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::TextDelta("No rewrite was applied.".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ],
        ])),
    });
    let admission = note_start::start_selected_with_factory(
        &f.store,
        sessions(&f),
        confirmation(&f, &record).into(),
        |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
    )
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new run")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message| matches!(message,
        ModelMessage::ToolResult { call_id, is_error: true, .. } if call_id == "rewrite"
    )));
    drop(requests);
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select sum(revision) from note_pages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select sum(revision) from note_blocks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from agent_note_applications", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
