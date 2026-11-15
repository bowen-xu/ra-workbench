//! The original Core shared Agent executes a file task with independent policy.
use super::*;
use crate::{
    agent::{
        context_session::{
            resource_grants,
            sources::{Access, Resource},
        },
        model::{
            AgentHost, AgentModelResolver, PreparedAgentModel,
            responses_transport::PreviewResponsesWire,
        },
        repository::submissions::SqliteSubmissionStore,
        service::AgentSessionService,
    },
    ai::{
        docling_execution::DoclingExecution,
        generation_support::{ProviderInput, ProviderMessage, ProviderMessageRole},
        paper_reading_contracts::PaperAiReadingContextItem,
        reading_contracts::LlmModelSelection,
    },
};
use research_agent_core::{
    conversation::ConversationId,
    model::{ModelCapabilities, ModelFuture},
    protocol::{EventPayload, ModelIdentity, TurnOutcome},
    submission::{SubmissionInput, SubmissionStore},
};
use std::sync::Mutex;
use uuid::Uuid;

struct Wire {
    sessions: Arc<AgentSessionService>,
    conversation: ConversationId,
    path: String,
    content: Value,
    apply: bool,
    requests: Mutex<Vec<Value>>,
}

impl PreviewResponsesWire for Wire {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert!(
            body.to_string()
                .contains("Final chat text is not a write receipt.")
        );
        assert!(
            body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "research_apply_patch")
        );
        assert!(self.sessions.tasks().has_active(self.conversation).unwrap());
        let mut requests = self.requests.lock().unwrap();
        requests.push(body.clone());
        let index = requests.len();
        let item = match (self.apply, index) {
            (true, 1) => {
                json!({"id":"read-target", "type":"function_call", "call_id":"read-target", "name":"research_read_file", "arguments":json!({"path":self.path}).to_string()})
            }
            (true, 2) => {
                assert!(body.to_string().contains("read-target"));
                let patch = format!(
                    "*** Begin Patch\n*** Update File: {}\n@@\n+{}\n*** End Patch",
                    self.path, self.content
                );
                json!({"id":"write-target", "type":"function_call", "call_id":"write-target", "name":"research_apply_patch", "arguments":json!({"patch":patch}).to_string()})
            }
            (true, 3) | (false, 1) => {
                if self.apply {
                    assert!(body.to_string().contains("applied"), "{body}");
                }
                json!({"id":"final", "type":"message", "role":"assistant", "status":"completed", "content":[{"type":"output_text", "text":"Synthetic model says saved."}]})
            }
            _ => panic!("unexpected model request {index}"),
        };
        let body: String = [
            json!({"type":"response.created", "response":{"id":format!("analysis-{index}"), "model":"gpt-5.6-terra"}}),
            json!({"type":"response.output_item.done", "output_index":0, "item":item}),
            json!({"type":"response.completed", "response":{"id":format!("analysis-{index}"), "model":"gpt-5.6-terra", "status":"completed", "output":[item]}}),
        ].into_iter().map(|event| format!("data: {event}\n\n")).collect();
        Box::pin(async move {
            Ok(hyper::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::from(body))
                .unwrap()
                .into())
        })
    }
}

fn seed_run(f: &Fixture, conversation: Uuid, request: Uuid) {
    let mut db = f.store.connection().unwrap();
    let sha = "a".repeat(64);
    db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('00000000-0000-4000-8000-000000000201','paper_overview','generating','openai','gpt-5.6-terra','pipeline','prompt',1,'goal',?1,1,1000,1000,1000,'now','now')", [&sha]).unwrap();
    db.execute("insert into ai_scope_items(id,run_id,ordinal,item_kind,object_id,parent_object_id,content_sha256,snapshot_hash,created_at) values('scope','00000000-0000-4000-8000-000000000201',0,'attachment','selected','paper',?1,?1,'now')", [&sha]).unwrap();
    db.execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values('00000000-0000-4000-8000-000000000201',?1,?2,'paper_overview')", rusqlite::params![conversation.to_string(), request.to_string()]).unwrap();
    let run =
        crate::ai::structured_generation_support::StructuredGenerationFacts::new(&db, &f.store)
            .run(
                conversation,
                request,
                ["paper_overview", "paper_profile_classification"],
            )
            .unwrap()
            .unwrap();
    // Freeze the original persisted run/evidence shape before selecting Native.
    // The actual Agent and all resource writes below use the Core-only store.
    crate::ai::generation_support::persist_test_docling_evidence(
        &mut db,
        &run,
        &[PaperAiReadingContextItem {
            id: "P1-T1".into(),
            paper_id: "paper".into(),
            attachment_id: "selected".into(),
            content_sha256: sha,
            parse_run_id: "prepare-parse".into(),
            raw_ref: "#/texts/0".into(),
            kind: "text".into(),
            label: None,
            page_index: Some(0),
            regions: vec![],
            charspan: None,
            source_text_hash: Some(crate::ai::generation_support::hash_text(
                "Prepared evidence Ω",
            )),
            text: Some("Prepared evidence Ω".into()),
            table_json: None,
        }],
        &Default::default(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package and development model wire"]
async fn actual_package_structured_submission_uses_core_agent_file_tools_and_exact_applied_receipt()
{
    for (apply, frozen) in [(true, true), (false, true), (true, false)] {
        let (catalog, key) = actual_package();
        let mut f = Fixture::new();
        paper_preparation::seed(&f);
        if !frozen {
            crate::agent::skills::bundled::install_defaults(
                f.store.agent_configuration_directory(),
            )
            .unwrap();
        }
        let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
        let conversation = Uuid::new_v4();
        let id = ConversationId::from_uuid(conversation);
        sessions.create(id).await.unwrap();
        let request = Uuid::new_v4();
        seed_run(&f, conversation, request);
        let (target, grants) = {
            let db = f.store.connection().unwrap();
            let target =
                crate::structured_resources::find_by_paper(&db, "paper", "structuredReading")
                    .unwrap();
            let grants = resource_grants::grants(
                &db,
                &[
                    Resource::Paper {
                        resource_id: "paper".into(),
                        attachment_id: Some("selected".into()),
                        page_index: None,
                    },
                    Resource::StructuredObject {
                        resource_id: target.id.clone(),
                        access: Some(Access::ReadWrite),
                    },
                ],
                None,
            )
            .unwrap();
            (target, grants)
        };
        let model = ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: Some("synthetic-wire-v1".into()),
        };
        let submissions = SqliteSubmissionStore::new(f.store.shared_agent_connection());
        let admitted = if frozen {
            Some(
                submissions
                    .put(
                        conversation,
                        request,
                        SubmissionInput {
                            text: "Synthetic frozen extraction task Ω".into(),
                            user_input: vec![],
                            context: grants,
                            model: model.clone(),
                            reasoning_effort: None,
                            service_tier: None,
                            instructions: vec![],
                            instruction_configs: vec![],
                            skill_instructions: vec![],
                            skills: vec![],
                            remote_compaction_v2: None,
                        },
                    )
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        f.bind_actual_package(catalog, key);
        assert!(f.store.generation_policy_source().is_none());
        assert!(f.store.paper_reading_source().is_none());
        let content = json!({"kind":"paper_overview_unit_v1", "schemaVersion":1, "profileId":"generic_overview_v1", "fields":[
            {"field":"research_question", "status":"not_found", "statements":[]},
            {"field":"approach", "status":"not_found", "statements":[]},
            {"field":"key_result", "status":"not_found", "statements":[]},
            {"field":"conclusion", "status":"not_found", "statements":[]},
            {"field":"limitation", "status":"not_found", "statements":[]},
            {"field":"open_question", "status":"not_found", "statements":[]}
        ]});
        let wire = Arc::new(Wire {
            sessions: sessions.clone(),
            conversation: id,
            path: target.source_uri,
            content: content.clone(),
            apply,
            requests: Mutex::new(vec![]),
        });
        let model_wire = wire.clone();
        let resolver = AgentModelResolver::for_preview(
            &f.store,
            Arc::new(move |host, selection| {
                assert!(matches!(host, AgentHost::Review));
                assert!(selection.is_some());
                PreparedAgentModel::for_preview(
                    model.clone(),
                    None,
                    None,
                    ModelCapabilities {
                        vision: false,
                        tools: true,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                    model_wire.clone(),
                    Uuid::from_u128(0xa11ce),
                )
            }),
        )
        .unwrap();
        let execution = DoclingExecution::new(f.store.clone(), resolver, Some(sessions.clone()));
        let prepared = execution
            .prepare_model(&LlmModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
            })
            .await
            .unwrap();
        let result = execution
            .execute(
                prepared,
                ProviderInput {
                    audit_text: "Legacy audit".into(),
                    system_text: Some("Legacy input".into()),
                    messages: vec![ProviderMessage {
                        role: ProviderMessageRole::User,
                        text: "Legacy excerpt".into(),
                        images: vec![],
                    }],
                },
                Some((conversation, request)),
                None,
            )
            .await;
        if result.is_err() && apply {
            for body in wire.requests.lock().unwrap().iter() {
                if let Some(input) = body["input"].as_array() {
                    for item in input
                        .iter()
                        .filter(|item| item["type"] == "function_call_output")
                    {
                        eprintln!("synthetic tool result {item}");
                    }
                }
            }
        }
        if apply {
            assert_eq!(
                serde_json::from_str::<Value>(&result.unwrap().content).unwrap(),
                content
            );
        } else {
            let error = result
                .err()
                .expect("chat text must not act as a saved receipt");
            assert_eq!(error.code(), "invalid_input");
            assert!(error.to_string().contains("本轮目标尚有未成功应用的修改"));
        }
        assert_eq!(
            wire.requests.lock().unwrap().len(),
            if apply { 3 } else { 1 }
        );
        assert!(!sessions.tasks().has_active(id).unwrap());
        let record = submissions.get(conversation, request).await.unwrap();
        let history = sessions.read_since(id, 0, 200).await.unwrap();
        assert_eq!(history.items.iter().filter(|event| matches!(event.payload(), EventPayload::TurnStarted {turn_id, ..} if *turn_id==record.turn_id)).count(), 1);
        assert_eq!(history.items.iter().filter(|event| matches!(event.payload(), EventPayload::TurnFinished {turn_id, outcome:TurnOutcome::Completed} if *turn_id==record.turn_id)).count(), 1);
        if let Some(admitted) = admitted {
            assert_eq!(record.input, admitted.input);
        } else {
            assert!(!record.input.skills.is_empty());
            assert!(!record.input.user_input.is_empty());
        }
        let db = f.store.connection().unwrap();
        let saved = crate::structured_resources::get(&db, &target.id).unwrap();
        assert_eq!(saved.revision, i64::from(apply));
        if apply {
            assert_eq!(saved.content, Some(content.clone()));
        }
        drop(db);
        sessions.shutdown().await.unwrap();
        f.service.shutdown().await.unwrap();
        if apply {
            drop(execution);
            drop(wire);
            drop(submissions);
            drop(sessions);
            f.store.shared_agent_connection().shutdown().unwrap();
            drop(f.store);
            f.store = AppStore::open_with_paths(
                crate::storage::AppStorePaths {
                    data_directory: f._data.path().into(),
                    agent_configuration_directory: f._data.path().join("configuration"),
                },
                crate::storage::AppStorePorts::with_events(
                    crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
                    crate::storage::AppStoreSources::default(),
                ),
            )
            .unwrap();
            let saved =
                crate::structured_resources::get(&f.store.connection().unwrap(), &target.id)
                    .unwrap();
            assert_eq!(saved.revision, 1);
            assert_eq!(saved.content, Some(content));
            let restored = SqliteSubmissionStore::new(f.store.shared_agent_connection())
                .get(conversation, request)
                .await
                .unwrap();
            assert_eq!(restored.input, record.input);
            let restored_sessions = AgentSessionService::from_app_store(&f.store).unwrap();
            assert!(!restored_sessions.tasks().has_active(id).unwrap());
            let restored_history = restored_sessions.read_since(id, 0, 200).await.unwrap();
            assert_eq!(restored_history.items, history.items);
            restored_sessions.shutdown().await.unwrap();
        }
    }
}
