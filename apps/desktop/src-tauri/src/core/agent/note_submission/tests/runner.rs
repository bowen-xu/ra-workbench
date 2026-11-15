//! Exercise the real shared runner/store with a synthetic in-process transport.
use super::*;
use crate::agent::{
    note_start,
    repository::SqliteConversationStore,
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use research_agent_core::{model::*, protocol::TurnOutcome};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Stream(VecDeque<ModelStreamEvent>);
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
pub(super) struct Transport {
    pub(super) requests: Mutex<Vec<ModelRequest>>,
    pub(super) responses: Mutex<VecDeque<Vec<ModelStreamEvent>>>,
}

#[tokio::test]
async fn note_rewrite_proposal_is_journaled_and_recovers_without_applying_or_replaying() {
    let f = fixture(true);
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let record = prepare(&f.store, &f.models, input).await.unwrap();
    let arguments = serde_json::json!({"sourceId":record.input.context[0].source_id,"edits":[
        {"blockId":"selected","content":{"kind":"inline","items":[{"type":"text","text":"Reworded synthetic note","styles":["italic"]}]}}
    ]});
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([
            vec![
                ModelStreamEvent::Started {
                    response_id: "proposal".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "rewrite-1".into(),
                    name: "research_propose_note_rewrite".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "rewrite-1".into(),
                    delta: arguments.to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "rewrite-1".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ],
            vec![
                ModelStreamEvent::Started {
                    response_id: "answer".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::TextDelta("An unapplied rewrite is ready for review.".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ],
        ])),
    });
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            f.store.shared_agent_connection(),
            "note-rewrite-fixture".into(),
        )
        .unwrap(),
    )));
    let confirmed = || note_start::StartRequest {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        page_id: f.page,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: crate::agent::execution_profile::AgentExecutionMode::DeepSeekNonThinking,
    };
    let admitted = note_start::start_with_factory(&f.store, sessions, confirmed(), |_, _| async {
        Ok(transport.clone() as Arc<dyn ModelTransport>)
    })
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("expected first admission")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let result = {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "research_propose_note_rewrite")
        );
        requests[1]
            .messages
            .iter()
            .find_map(|message| match message {
                ModelMessage::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .expect("proposal provided to continuation")
    };
    let proposal: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(proposal["kind"], "noteInlineRewrite");
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(
        proposal["contextSnapshotId"],
        record.context_snapshot_id.to_string()
    );
    assert_eq!(
        proposal["edits"][0]["before"]["items"][0]["text"],
        "Frozen note evidence"
    );
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    {
        let db = reopened.connection().unwrap();
        let (revision,content):(i64,String) = db.query_row("select n.revision,b.content_json from note_pages n join note_blocks b on b.page_id=n.id where b.id='selected'",[],|r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(revision, 0);
        assert!(content.contains("Frozen note evidence"));
        assert!(!content.contains("Reworded"));
        assert_eq!(
            db.query_row("select count(*) from note_revisions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let saved: i64 = db.query_row("select count(*) from agent_items where json_extract(payload_json,'$.payload.type')='textDelta' and json_extract(payload_json,'$.payload.text')=?1",[&result],|r| r.get(0)).unwrap();
        assert_eq!(saved, 1, "one durable exact proposal result");
    }
    let reopened_sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            reopened.shared_agent_connection(),
            "note-rewrite-reopened".into(),
        )
        .unwrap(),
    )));
    let (_, projection) =
        crate::agent::repository::SqliteConversationCatalog::from_app_store(&reopened)
            .projection_snapshot(ConversationId::from_uuid(record.conversation_id))
            .await
            .unwrap();
    let item_id = *projection
        .turn(record.turn_id)
        .unwrap()
        .items
        .iter()
        .find(|(_, item)| {
            matches!(
                item.kind,
                research_agent_core::protocol::ItemKind::ToolResult { .. }
            )
        })
        .unwrap()
        .0;
    let review = crate::agent::note_review::review(
        &reopened,
        crate::agent::note_review::Scope {
            conversation_id: record.conversation_id,
            page_id: f.page,
            turn_id: record.turn_id,
            item_id,
        },
    )
    .await
    .unwrap();
    assert_eq!(review.proposal, proposal);
    assert!(review.baseline_matches);
    assert!(matches!(
        note_start::start_with_factory(&reopened, reopened_sessions, confirmed(), |_, _| async {
            panic!("reopened completed turn must not recreate a proposal")
        })
        .await
        .unwrap(),
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
}
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("bounded fixture call");
            Ok(Box::new(Stream(response.into())) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn prepared_note_runs_shared_source_tool_loop_without_live_scope_expansion_or_replay() {
    let f = fixture(true);
    let request_id = Uuid::new_v4();
    let mut input = request(&f, request_id);
    input.model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let record = prepare(&f.store, &f.models, input).await.unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([
            vec![
                ModelStreamEvent::Started {
                    response_id: "first".into(),
                    actual_model: record.input.model.clone(),
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "read-note".into(),
                    name: "research_read_source".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "read-note".into(),
                    delta: serde_json::json!({"sourceId":record.input.context[0].source_id})
                        .to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "read-note".into(),
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
                ModelStreamEvent::TextDelta("Synthetic answer".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ],
        ])),
    });
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            f.store.shared_agent_connection(),
            "note-runner-fixture".into(),
        )
        .unwrap(),
    )));
    let confirmed = || note_start::StartRequest {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        page_id: f.page,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: crate::agent::execution_profile::AgentExecutionMode::DeepSeekNonThinking,
    };
    let newer = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"New live note content","styles":[]}]}).to_string();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_blocks set content_json=?1 where id='selected'",
            [newer],
        )
        .unwrap();
    let first =
        note_start::start_with_factory(&f.store, sessions.clone(), confirmed(), |_, _| async {
            Ok(transport.clone() as Arc<dyn ModelTransport>)
        })
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = first else {
        panic!("first run must start")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert!(matches!(
        note_start::start_with_factory(&f.store, sessions, confirmed(), |_, _| async {
            panic!("existing turn must not prepare transport")
        })
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
            .any(|tool| tool.name == "research_read_source")
    );
    let result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            ModelMessage::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .expect("source result in continuation");
    assert!(result.contains("Frozen note evidence"));
    assert!(!result.contains("New live note content"));
    assert!(result.contains(&record.context_snapshot_id.to_string()));
    assert!(result.contains(&record.input.context[0].source_id));
}
