use super::*;
use crate::agent::repository::submissions;
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
#[path = "tests/auto_docling.rs"]
mod auto_docling;
#[path = "tests/discovery.rs"]
mod discovery;
#[path = "tests/file_id.rs"]
mod file_id;
#[path = "tests/frozen.rs"]
mod frozen;
#[path = "tests/structured.rs"]
mod structured;
fn fixture() -> (tempfile::TempDir, AppStore, Uuid) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Fixture','now','now')",[]).unwrap();
    let id = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values (?1,'Fixture')",
        [id.to_string()],
    )
    .unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    (root, store, id)
}
fn note(store: &AppStore, body: &str) -> String {
    let db = store.connection().unwrap();
    let alias = Uuid::new_v4().to_string();
    let id = crate::notes::insert_reading_fixture(&db, &alias, body);
    db.execute(
        "update reading_note_sources set paper_id_snapshot='paper' where page_id=?1",
        [&id],
    )
    .unwrap();
    // The retained-source fixture also created this link through migration 0188.
    // Reassign both identities so the intended same-paper exclusion stays valid.
    assert_eq!(db.execute(
        "update note_links set target_id='paper' where id=?1 and source_page_id=?2 and source_block_id is null and target_kind='paper' and target_id='missing-paper' and relation_type='derived_from'",
        params![format!("reading-origin:{alias}"), id],
    ).unwrap(), 1);
    id
}
fn tools(store: &AppStore, id: Uuid, resources: Vec<Resource>) -> ResourceTools {
    let mut db = store.connection().unwrap();
    let tx = db.transaction().unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "Explain sources".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: resource_grants::grants(
            &tx,
            &resources,
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap(),
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    };
    let record = submissions::insert_in(&tx, id, Uuid::new_v4(), input).unwrap();
    tx.commit().unwrap();
    ResourceTools {
        store: store.clone(),
        record,
        scope: None,
    }
}
fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: args,
    }
}
#[test]
fn a_note_group_discovers_new_notes_and_reads_latest_content_without_preloading_or_bypassing_exclusions()
 {
    let (_root, store, id) = fixture();
    let kept = note(&store, "first evidence");
    let excluded = note(&store, "excluded evidence");
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperNotes {
            resource_id: "paper".into(),
            excluded_note_ids: vec![excluded.clone()],
        }],
    );
    assert!(
        !serde_json::to_string(&tools.record.input.context)
            .unwrap()
            .contains("first evidence")
    );
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["total"], 1);
    let source_id = list["sources"][0]["sourceId"].as_str().unwrap();
    let first = tools
        .evaluate(&call(
            "research_read_source",
            json!({"sourceId":source_id,"maxChars":8000}),
        ))
        .unwrap();
    assert!(first.to_string().contains("first evidence"));
    let added = note(&store, "new evidence");
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["total"], 2);
    assert!(list.to_string().contains(&added));
    assert!(!list.to_string().contains(&excluded));
    let db = store.connection().unwrap();
    db.execute(
        "update note_pages set title='Changed title',revision=revision+1 where id=?1",
        [&kept],
    )
    .unwrap();
    drop(db);
    let second = tools
        .evaluate(&call("research_read_source", json!({"sourceId":source_id})))
        .unwrap();
    assert!(second.to_string().contains("Changed title"));
    assert!(!first.to_string().contains("Changed title"));
    assert!(first.get("revision").is_none());
    assert!(second.get("revision").is_none());
    let excluded_id = resource_grants::grants(
        &store.connection().unwrap(),
        &[Resource::Note {
            resource_id: Uuid::parse_str(&excluded).unwrap(),
        }],
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap()
    .remove(0)
    .source_id;
    assert_eq!(
        tools.evaluate(&call(
            "research_read_source",
            json!({"sourceId":excluded_id})
        )),
        Err(Error::SourceUnavailable)
    );
    let found = tools
        .evaluate(&call(
            "research_search_sources",
            json!({"query":"new evidence"}),
        ))
        .unwrap();
    assert_eq!(found["total"], 1);
    assert!(found["matches"][0].get("revision").is_none());
    assert_eq!(
        tools
            .evaluate(&call(
                "research_search_sources",
                json!({"query":"excluded evidence"})
            ))
            .unwrap()["total"],
        0
    );
}
#[test]
fn rejects_foreign_exclusions_and_archived_conversations() {
    let (_root, store, id) = fixture();
    let excluded = note(&store, "private");
    let db = store.connection().unwrap();
    db.execute(
        "update reading_note_sources set paper_id_snapshot='other' where page_id=?1",
        [&excluded],
    )
    .unwrap();
    assert_eq!(
        resource_grants::grants(
            &db,
            &[Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: vec![excluded]
            }],
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::InvalidInput)
    );
    drop(db);
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperMetadata {
            resource_id: "paper".into(),
        }],
    );
    store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert_eq!(
        tools.evaluate(&call("research_list_sources", json!({}))),
        Err(Error::ConversationUnavailable)
    );
}
#[tokio::test]
async fn schemas_route_live_reads_through_the_existing_registration() {
    let (_root, store, id) = fixture();
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperMetadata {
            resource_id: "paper".into(),
        }],
    );
    let mut record = tools.record.clone();
    record
        .input
        .context
        .push(research_agent_core::submission::ContextText {
            source_id: "frozen-excerpt".into(),
            kind: research_agent_core::submission::ContextSourceKind::Paper,
            resource_id: "paper".into(),
            revision: "old-frozen-version".into(),
            anchor: None,
            text: "Previously authorized immutable source excerpt.".into(),
        });
    let frozen = crate::agent::source_tools::OwnedSubmissionTools::new(record.clone());
    let registered = registration(
        store.clone(),
        record,
        OwnedToolRegistration::new(frozen.definitions(), Arc::new(frozen)),
    );
    let (definitions, executor) = registered.into_parts();
    assert!(!executor.requires_result_receipt(&call("research_list_sources", json!({}))));
    assert!(!executor.requires_result_receipt(&call(
        "research_read_source",
        json!({"sourceId":"frozen-excerpt"})
    )));
    assert!(executor.requires_result_receipt(&call("unknown_tool", json!({}))));
    assert_eq!(definitions.len(), 6);
    let names = definitions
        .iter()
        .map(|tool| &tool.name)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        names.len(),
        definitions.len(),
        "live source schemas replace frozen schemas by name"
    );
    assert!(
        definitions
            .iter()
            .any(|tool| tool.name == "research_read_file")
    );
    assert_eq!(
        definitions
            .iter()
            .find(|tool| tool.name == "research_apply_patch")
            .unwrap()
            .parameters["required"],
        json!(["patch"])
    );
    let output = executor
        .authorize_and_execute(call("research_list_sources", json!({})))
        .await
        .unwrap();
    assert!(!output.is_error);
    let listed: Value = serde_json::from_str(&output.content).unwrap();
    let output = executor
        .authorize_and_execute(call(
            "research_read_source",
            json!({"sourceId":listed["sources"][0]["sourceId"]}),
        ))
        .await
        .unwrap();
    assert!(!output.is_error);
    assert!(output.content.contains("Fixture"));
    let frozen = executor
        .authorize_and_execute(call(
            "research_read_source",
            json!({"sourceId":"frozen-excerpt"}),
        ))
        .await
        .unwrap();
    assert!(!frozen.is_error);
    assert!(
        frozen
            .content
            .contains("Previously authorized immutable source excerpt.")
    );
}

#[tokio::test]
async fn shared_agent_loop_recovers_oversized_read_and_records_current_content_without_preloading_bodies()
 {
    use crate::agent::{
        execution::AgentExecution,
        repository::SqliteSubmissionStore,
        service::{AgentSessionService, SubmissionAdmission},
        tasks::TaskCompletion,
    };
    use research_agent_core::{conversation::ConversationId, model::*, runner::RunLimits};
    use std::{collections::VecDeque, sync::Mutex};
    struct Stream(VecDeque<ModelStreamEvent>);
    impl ModelStream for Stream {
        fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
            Box::pin(async move { Ok(self.0.pop_front()) })
        }
    }
    struct Transport {
        requests: Mutex<Vec<ModelRequest>>,
        responses: Mutex<VecDeque<Vec<ModelStreamEvent>>>,
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
                let events = self.responses.lock().unwrap().pop_front().unwrap();
                Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>)
            })
        }
    }
    let (_root, store, id) = fixture();
    let page = note(&store, "first body");
    let tools = tools(
        &store,
        id,
        vec![Resource::Note {
            resource_id: Uuid::parse_str(&page).unwrap(),
        }],
    );
    let source = tools.record.input.context[0].source_id.clone();
    store.connection().unwrap().execute("update note_blocks set plain_text='current body',content_json=?1 where page_id=?2",params![json!({"kind":"inline","items":[{"type":"text","text":"current body","styles":[]}]}).to_string(),page]).unwrap();
    let model = tools.record.input.model.clone();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(
            vec![
                vec![
                    ModelStreamEvent::Started {
                        response_id: "oversize".into(),
                        actual_model: model.clone(),
                    },
                    ModelStreamEvent::ToolCallStarted {
                        call_id: "oversize".into(),
                        name: "research_read_source".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: "oversize".into(),
                        delta: json!({"sourceId":source,"maxChars":13500}).to_string(),
                    },
                    ModelStreamEvent::ToolCallCompleted {
                        call_id: "oversize".into(),
                    },
                    ModelStreamEvent::Completed {
                        reason: FinishReason::ToolCalls,
                        end_turn: None,
                    },
                ],
                vec![
                    ModelStreamEvent::Started {
                        response_id: "first".into(),
                        actual_model: model.clone(),
                    },
                    ModelStreamEvent::ToolCallStarted {
                        call_id: "read".into(),
                        name: "research_read_source".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: "read".into(),
                        delta: json!({"sourceId":source,"maxChars":8000}).to_string(),
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
                        actual_model: model.clone(),
                    },
                    ModelStreamEvent::TextDelta("Read current body".into()),
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        end_turn: None,
                    },
                ],
            ]
            .into(),
        ),
    });
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        compaction_strategy: None,
        model,
        transport: transport.clone(),
        host_tools: registration(
            store.clone(),
            tools.record.clone(),
            OwnedToolRegistration::new(
                vec![],
                Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
            ),
        ),
        system_instructions: crate::agent::host_instructions::GENERAL.into(),
        max_output_tokens: 2000,
        max_input_bytes: 100_000,
        limits: RunLimits {
            model_calls: Some(3),
            tool_calls: Some(3),
            output_bytes: Some(10_000),
            tool_output_bytes: 10_000,
            response: StreamLimits {
                text_bytes: 10_000,
                argument_bytes: 10_000,
                tool_calls: 3,
            },
        },
    });
    let admitted = execution
        .start(
            service,
            Arc::new(SqliteSubmissionStore::new(store.shared_agent_connection())),
            ConversationId::from_uuid(id),
            tools.record.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("started")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let text = |request: &ModelRequest| {
        request
            .messages
            .iter()
            .map(|message| match message {
                ModelMessage::CanonicalOutput(item) => match &item.normalized {
                    Some(research_agent_core::model::ModelOutputItem::Message(message)) => {
                        message.text.clone()
                    }
                    _ => String::new(),
                },
                ModelMessage::System(text)
                | ModelMessage::Developer(text)
                | ModelMessage::ToolResult { content: text, .. } => text.clone(),
                ModelMessage::User { content: parts, .. }
                | ModelMessage::Assistant { content: parts, .. } => parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let first = text(&requests[0]);
    let second = text(&requests[2]);
    assert!(requests[1].messages.iter().any(|message| matches!(message,
        ModelMessage::ToolResult { call_id, content, is_error: true }
        if call_id == "oversize" && content.contains("8000") && content.contains("nextChar"))));
    assert!(!text(&requests[1]).contains("current body"));
    assert!(!first.contains("current body"));
    assert!(!first.contains("first body"));
    assert!(second.contains("current body"));
    assert_eq!(
        requests[0]
            .tools
            .iter()
            .filter(|tool| tool.name == "research_read_source")
            .count(),
        1
    );
    let db = store.connection().unwrap();
    let count: u64 = db.query_row("select count(*) from agent_items where conversation_id=?1 and payload_json like '%invalid_tool_arguments%'", [id.to_string()], |row| row.get(0)).unwrap();
    assert!(count > 0, "argument feedback survives journal recovery");
}

#[path = "tests/compact.rs"]
mod compact;
#[path = "tests/dynamic.rs"]
mod dynamic;

#[test]
fn note_codec_failure_keeps_safe_identity_without_claiming_source_changed() {
    let request = Uuid::new_v4();
    let error = note_read_error(
        "MODEL_NOTE_TEXT_VIEW_UNSTABLE: private details".into(),
        "page".into(),
        request,
    );
    let message = error.to_string();
    assert!(message.contains("MODEL_NOTE_TEXT_VIEW_UNSTABLE"));
    assert!(message.contains(&request.to_string()));
    assert!(message.contains("page=page"));
    assert!(!message.contains("private details"));
    assert!(!message.contains("unavailable or changed"));
    assert_eq!(
        note_read_error("private storage failure".into(), "page".into(), request),
        Error::SourceUnavailable
    );
}
