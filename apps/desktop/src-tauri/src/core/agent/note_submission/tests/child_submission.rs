use super::start::{confirmation, saved, sessions};
use super::*;
use crate::agent::{
    note_start,
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use research_agent_core::model::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[tokio::test]
async fn note_child_submission_commit_unknown_stops_and_reopens_original_call() {
    use super::runner::Transport;
    let f = fixture_with_owner(true, true);
    let record = saved(&f).await;
    f.store.connection().unwrap().execute_batch("create table child_submit_parent(id integer primary key);
      create table child_submit_deferred(id integer references child_submit_parent(id) deferrable initially deferred);
      create trigger child_submit_unknown after insert on data_sql_operation_receipts when new.kind='agent.child.submission'
      begin insert into child_submit_deferred(id) values(9); end;").unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([vec![
            ModelStreamEvent::Started {
                response_id: "note-child-submission".into(),
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
        panic!("expected started parent")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert!(f.store.connection().is_err());
    let Fixture {
        root,
        store,
        models,
        conversation,
        ..
    } = f;
    drop(owned);
    drop(transport);
    drop(models);
    drop(store);
    let db = crate::data_interface::Database::open(
        Connection::open(root.path().join("fixture.sqlite")).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let reopened = AppStore::for_test_database(db.clone(), root.path().join("data/objects"));
    let sessions = AgentSessionService::from_app_store(&reopened).unwrap();
    let history = sessions
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    assert!(history.has_unknown_tool_outcome());
    let turn = history.turn(record.turn_id).unwrap();
    let call_id = *turn.child_creation_intents.keys().next().unwrap();
    let child = *turn.child_creation_intents[&call_id].id.as_uuid();
    let catalog = crate::agent::repository::SqliteConversationCatalog::from_app_store(&reopened);
    let status = crate::agent::child_creation::status::execute(
        &sessions,
        &catalog,
        crate::agent::child_creation::status::Request {
            schema_version: 1,
            conversation_id: conversation,
            turn_id: record.turn_id,
            call_id,
        },
    )
    .await
    .unwrap();
    let wire = serde_json::to_value(status).unwrap();
    assert_eq!(wire["creation"]["status"]["type"], "committed");
    assert_eq!(
        wire["submission"]["requestId"],
        research_agent_core::conversation::child_submission_request_id(child).to_string()
    );
    assert_eq!(wire["submission"]["record"], serde_json::Value::Null);
    assert!(wire.get("cleanup").is_none());
    let sql = db.lock().unwrap();
    let counts: (u64,u64) = sql.query_row("select (select count(*) from agent_child_cleanup_intents), (select count(*) from data_sql_operation_receipts where kind='agent.conversation.archive')", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
}
