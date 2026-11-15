use super::*;
use crate::{
    agent::model::{AgentHost, AgentModelResolver, responses_transport::PreviewResponsesWire},
    ai::{
        generation_hooks::{PaperCompletionFuture, PreparedPaperWorkflow},
        generation_support::{ProviderMessage, ProviderMessageRole},
    },
    runtime_events::RuntimeEvents,
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_agent_core::{
    model::ModelFuture,
    protocol::{ModelIdentity, TurnOutcome},
    submission::ContextText,
    user_input::UserInput,
};
use std::{fs::OpenOptions, io::Write, path::Path};

// Synthetic receipt data only. The trace belongs to this isolated fixture; no
// production setter, provider catalog, credential or network server is involved.
fn trace(store: &AppStore, stage: &str) {
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(store.content_directory().join("synthetic-hook-trace.txt"))
            .unwrap(),
        "{stage}"
    )
    .unwrap();
}

fn stages(store: &AppStore) -> Vec<String> {
    std::fs::read_to_string(store.content_directory().join("synthetic-hook-trace.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

struct Hook {
    store: AppStore,
    conversation: Uuid,
    request: Uuid,
    fail: bool,
}

impl PreparedPaperWorkflow for Hook {
    fn kind(&self) -> &str {
        "structuredReading"
    }
    fn grants(&self) -> &[ContextText] {
        &[]
    }
    fn prompt_text(&self) -> &str {
        "synthetic frozen question"
    }
    fn prompt_user_input(&self) -> &[UserInput] {
        &[]
    }
    fn decorate(
        &self,
        input: &mut ProviderInput,
        submission: &SubmissionInput,
    ) -> Result<(), AiError> {
        assert_eq!(submission.text, if self.fail { "fail" } else { "success" });
        assert_eq!(stages(&self.store), ["prepare"]);
        trace(&self.store, "decorate");
        input.system_text = Some("synthetic domain decoration".into());
        Ok(())
    }
    fn complete<'a>(
        &'a self,
        store: &'a AppStore,
        conversation: Uuid,
        request: Uuid,
        mut response: VisionGenerationResponse,
    ) -> PaperCompletionFuture<'a> {
        Box::pin(async move {
            assert_eq!((conversation, request), (self.conversation, self.request));
            assert_eq!(stages(store), ["prepare", "decorate", "model"]);
            assert_eq!(response.content, "synthetic model answer");
            trace(store, "complete");
            if self.fail {
                Err(AiError::InvalidInput(
                    "synthetic completion receipt rejected".into(),
                ))
            } else {
                response.content = "synthetic domain receipt".into();
                Ok(response)
            }
        })
    }
}

fn prepare(
    store: &AppStore,
    conversation: Uuid,
    request: Uuid,
    frozen: Option<&SubmissionInput>,
) -> Result<Option<Box<dyn PreparedPaperWorkflow>>, AiError> {
    let frozen = frozen.expect("the real submission must already be persisted");
    trace(store, "prepare");
    Ok(Some(Box::new(Hook {
        store: store.clone(),
        conversation,
        request,
        fail: frozen.text == "fail",
    })))
}

struct Wire {
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    conversation: ConversationId,
    turn: Uuid,
    receipt: Mutex<Option<crate::agent::tasks::TaskReceipt>>,
}

impl PreviewResponsesWire for Wire {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        Box::pin(async move {
            let body: serde_json::Value =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            assert!(body.to_string().contains("synthetic domain decoration"));
            assert_eq!(stages(&self.store), ["prepare", "decorate"]);
            assert!(self.sessions.tasks().has_active(self.conversation).unwrap());
            let receipt = self
                .sessions
                .tasks()
                .find_turn_receipt(self.conversation, self.turn)
                .unwrap()
                .expect("the real session owns this admitted turn");
            assert!(!receipt.is_finished());
            *self.receipt.lock().unwrap() = Some(receipt);
            let history = self
                .sessions
                .read_since(self.conversation, 0, 100)
                .await
                .unwrap();
            assert_eq!(history.items.iter().filter(|event| matches!(
                event.payload(), EventPayload::TurnStarted { turn_id, .. } if *turn_id == self.turn
            )).count(), 1);
            assert!(
                !history
                    .items
                    .iter()
                    .any(|event| matches!(event.payload(), EventPayload::TurnFinished { .. }))
            );
            trace(&self.store, "model");
            let frames = [
                json!({"type":"response.created","response":{"id":"synthetic-response"}}),
                json!({"type":"response.output_item.added","item":{"type":"message","id":"synthetic-message","role":"assistant","phase":"final_answer","content":[]}}),
                json!({"type":"response.output_text.delta","item_id":"synthetic-message","delta":"synthetic model answer"}),
                json!({"type":"response.output_item.done","item":{"type":"message","id":"synthetic-message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"synthetic model answer"}]}}),
                json!({"type":"response.completed","response":{"id":"synthetic-response","end_turn":true}}),
            ];
            let body = frames
                .into_iter()
                .map(|frame| format!("data: {frame}\n\n"))
                .collect::<String>();
            Ok(hyper::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::from(body))
                .unwrap()
                .into())
        })
    }
}

fn store(root: &Path) -> AppStore {
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: root.to_owned(),
            agent_configuration_directory: root.join("configuration"),
        },
        AppStorePorts::with_events(
            RuntimeEvents::new(|_, _| Ok(())),
            AppStoreSources {
                native_note_trace: None,
                writing_context_source: None,
                writing_resource_source: Some(
                    crate::agent::writing_resources::test_sources::source(),
                ),
                paper_metadata_source: None,
                retrieval_trace_source: None,
                retrieval_preview_source: None,
                semantic_index_source: None,
                paper_portrait_source: None,
                context_catalog_source: crate::context_catalog_app::source(),
                generation_policy_source: None,
                paper_generation_source: None,
                paper_reading_source: None,
                paper_analysis_source: None,
                candidate_recovery_source: None,
                writing_compile_feedback_source: None,
                #[cfg(unix)]
                writing_workspace_source: None,
                #[cfg(target_os = "macos")]
                writing_workspace_compile_source: None,
            },
        ),
    )
    .unwrap()
}

#[tokio::test]
async fn domain_hooks_finish_inside_the_original_owned_turn() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
        let conversation = Uuid::new_v4();
        let id = ConversationId::from_uuid(conversation);
        sessions.create(id).await.unwrap();
        let request = Uuid::new_v4();
        let model = ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: Some("synthetic-wire-v1".into()),
        };
        let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
        let record = submissions
            .put(
                conversation,
                request,
                SubmissionInput {
                    text: if fail { "fail" } else { "success" }.into(),
                    model: model.clone(),
                    user_input: vec![],
                    skill_instructions: vec![],
                    context: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills: vec![],
                    reasoning_effort: None,
                    service_tier: None,
                    remote_compaction_v2: None,
                },
            )
            .await
            .unwrap();
        let wire = Arc::new(Wire {
            store: store.clone(),
            sessions: sessions.clone(),
            conversation: id,
            turn: record.turn_id,
            receipt: Mutex::new(None),
        });
        let factory_wire = wire.clone();
        let resolver = AgentModelResolver::for_preview(
            &store,
            Arc::new(move |host, selection| {
                assert!(matches!(host, AgentHost::Reading));
                assert!(selection.is_none());
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
                    factory_wire.clone(),
                    Uuid::from_u128(0xa11ce),
                )
            }),
        )
        .unwrap();
        let prepared = resolver.prepare(AgentHost::Reading, None).await.unwrap();
        let workflow = prepare(&store, conversation, request, Some(&record.input)).unwrap();
        let transport_submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
        let result = call_with_workflow(
            store.clone(),
            sessions.clone(),
            prepared,
            ProviderInput {
                audit_text: "synthetic frozen question".into(),
                system_text: None,
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    text: "synthetic frozen question".into(),
                    images: vec![],
                }],
            },
            conversation,
            request,
            CancellationSignal::default(),
            transport_submissions,
            workflow,
        )
        .await;
        trace(&store, "returned-after-wait");
        assert_eq!(
            stages(&store),
            [
                "prepare",
                "decorate",
                "model",
                "complete",
                "returned-after-wait"
            ]
        );
        if fail {
            assert!(
                matches!(result, Err(AiError::InvalidInput(message)) if message == "synthetic completion receipt rejected")
            );
        } else {
            assert_eq!(result.unwrap().content, "synthetic domain receipt");
        }
        let receipt = wire.receipt.lock().unwrap().clone().unwrap();
        assert_eq!(receipt.turn_id(), Some(record.turn_id));
        assert!(receipt.is_finished());
        assert_eq!(
            receipt.wait().await,
            if fail {
                TaskCompletion::Failed
            } else {
                TaskCompletion::Completed
            }
        );
        assert!(!sessions.tasks().has_active(id).unwrap());
        let history = sessions.read_since(id, 0, 100).await.unwrap();
        assert_eq!(
            history
                .items
                .iter()
                .filter(|event| matches!(event.payload(), EventPayload::TurnStarted { .. }))
                .count(),
            1
        );
        // Model execution has already durably finished before the domain receipt
        // is checked; a rejected receipt fails the same task without replaying it.
        assert_eq!(history.items.iter().filter(|event| matches!(event.payload(), EventPayload::TurnFinished { turn_id, outcome: TurnOutcome::Completed } if *turn_id == record.turn_id)).count(), 1);
        assert!(history.items.iter().any(|event| matches!(event.payload(), EventPayload::TextDelta { text, .. } if text == "synthetic model answer")));
        assert_eq!(
            submissions.get(conversation, request).await.unwrap().input,
            record.input
        );
    }
}
