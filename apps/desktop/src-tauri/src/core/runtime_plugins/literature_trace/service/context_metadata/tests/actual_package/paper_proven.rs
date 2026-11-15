//! Actual Core proof, replay and reopen using a real independently built package.
use super::*;
use crate::{
    agent::{
        model::{
            AgentModelPreparationError, AgentModelResolver, PreparedAgentModel,
            responses_transport::PreviewResponsesWire,
        },
        reading_submission,
    },
    ai::{
        paper_context::prepare,
        paper_reading_contracts::{PaperAiReadingImageAssetDraft, PrepareRequest},
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use research_agent_core::{model::ModelFuture, protocol::ModelIdentity};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use uuid::Uuid;
mod queue;

struct NoInference;
impl PreviewResponsesWire for NoInference {
    fn respond(&self, _: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        Box::pin(async { panic!("paper preparation must not call inference") })
    }
}
fn models(
    store: &AppStore,
    calls: Arc<AtomicUsize>,
    disabled: Arc<AtomicBool>,
) -> AgentModelResolver {
    models_with_wire(store, calls, disabled, Arc::new(NoInference))
}
fn models_with_wire(
    store: &AppStore,
    calls: Arc<AtomicUsize>,
    disabled: Arc<AtomicBool>,
    wire: Arc<dyn PreviewResponsesWire>,
) -> AgentModelResolver {
    AgentModelResolver::for_preview(
        store,
        Arc::new(move |host, selection| {
            assert!(matches!(host, crate::agent::model::AgentHost::Reading));
            calls.fetch_add(1, Ordering::SeqCst);
            if disabled.load(Ordering::SeqCst) {
                return Err(AgentModelPreparationError::Settings);
            }
            let selected = selection.ok_or(AgentModelPreparationError::Settings)?;
            let model = ModelIdentity {
                provider_id: selected.provider_id,
                model_id: selected.model_id,
                revision: Some("synthetic-wire-v1".into()),
            };
            let mut capabilities =
                crate::ai::agent_configuration::preview::capabilities_fixture(&model)?;
            // The synthetic provider declares the existing Reading vision class.
            // Core still intersects it with the reviewed host profile.
            capabilities.vision = true;
            PreparedAgentModel::for_preview(
                model,
                selected.reasoning_effort,
                selected.service_tier,
                capabilities,
                Arc::clone(&wire),
                Uuid::from_u128(0xa11ce),
            )
        }),
    )
    .unwrap()
}
#[tokio::test]
#[ignore = "requires a real literature package and the development model preparation port"]
async fn actual_package_prepares_proof_replays_without_model_and_restores_after_core_reopen() {
    round_trip(TraceMode::None).await;
}
#[tokio::test]
#[ignore = "requires a real literature package and the development model preparation port"]
async fn actual_package_consumes_paper_trace_once_and_replays_frozen_context_after_reopen() {
    round_trip(TraceMode::Seeded).await;
}
#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_generates_retrieval_then_submits_replays_reopens_and_reads_source() {
    round_trip(TraceMode::Generated).await;
}
#[tokio::test]
#[ignore = "requires an actual independent literature package and C ABI vector engine"]
async fn actual_package_queued_question_freezes_current_sources_reopens_starts_and_removes_once() {
    round_trip(TraceMode::GeneratedQueued).await;
}
#[derive(Clone, Copy, PartialEq)]
enum TraceMode {
    None,
    Seeded,
    Generated,
    GeneratedQueued,
}
async fn round_trip(mode: TraceMode) {
    let with_trace = mode != TraceMode::None;
    let generated = matches!(mode, TraceMode::Generated | TraceMode::GeneratedQueued);
    let paper = if mode == TraceMode::GeneratedQueued {
        Uuid::new_v4().to_string()
    } else {
        "paper".into()
    };
    let mut trace_id = String::from("paper-trace");
    let mut trace_context = String::from("[CN-S1] traced evidence Ω");
    let (catalog, key) = actual_package();
    let mut f = Fixture::for_paper(&paper);
    paper_preparation::seed_for_paper(&f, &paper);
    f.bind_actual_package(catalog.clone(), key.clone());
    assert!(f.store.paper_reading_source().is_none());
    let conversation = f.record.conversation_id;
    f.store.connection().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper',?2)", rusqlite::params![conversation.to_string(),paper]).unwrap();
    let queue = if mode == TraceMode::GeneratedQueued {
        Some(queue::QueueProof::enqueue(&f, &paper).await)
    } else {
        None
    };
    let reading =
        prepare::prepare_reading_context(&f.store, paper.clone(), vec!["selected".into()])
            .await
            .unwrap();
    let image = PaperAiReadingImageAssetDraft {
        context_item_id: reading.image_requests[0].context_item_id.clone(),
        data_url: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(b"\x89PNG\r\n\x1a\nproven-reading-image")
        ),
    };
    let snapshot = prepare::prepare_context(
        &f.store,
        PrepareRequest {
            paper_id: paper.clone(),
            attachment_ids: vec!["selected".into()],
            image_assets: vec![image],
        },
    )
    .await
    .unwrap();
    let mut request: reading_submission::PrepareRequest = serde_json::from_value(json!({
        "conversationId":conversation, "requestId":Uuid::new_v4(), "paperId":paper,
        "paperContextSnapshotId":snapshot.snapshot_id, "expectedContextHash":snapshot.context_hash,
        "text":"Read this paper Ω", "userInput":[], "retrieval":null,
        "model":{"providerId":"openai","modelId":"gpt-5.6-terra","reasoningEffort":null,"serviceTier":null}
    })).unwrap();
    if generated {
        let preview = super::retrieval_preview::generate(&f, &paper, &request.text).await;
        trace_id = preview.trace_id;
        trace_context = super::retrieval_preview::context(&f, &trace_id);
        request.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
            trace_id: trace_id.clone(),
            query: request.text.clone(),
        });
    } else if with_trace {
        use sha2::{Digest, Sha256};
        let context = "[CN-S1] traced evidence Ω";
        let hash = |text: &str| format!("{:x}", Sha256::digest(text.as_bytes()));
        f.store.connection().unwrap().execute(
            "insert into concept_retrieval_traces(id,paper_id,query_sha256,status,config_json,context_text,context_sha256,seed_count,edge_count,source_count,external_paper_count,created_at,expires_at) values('paper-trace','paper',?1,'previewed','{}',?2,?3,0,0,0,0,'now','9999-12-31T23:59:59Z')",
            rusqlite::params![hash(request.text.trim()),context,hash(context)],
        ).unwrap();
        request.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
            trace_id: "paper-trace".into(),
            query: request.text.clone(),
        });
    }
    let initial =
        reading_submission::prepare_status::read_with_epoch(&f.store, request.clone(), None)
            .await
            .unwrap();
    let initial = serde_json::to_value(initial).unwrap();
    assert_eq!(initial["status"]["type"], "unconfirmed");
    let epoch = Some(serde_json::from_value(initial["status"]["detail"]["epoch"].clone()).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let disabled = Arc::new(AtomicBool::new(false));
    let resolver = models(&f.store, Arc::clone(&calls), Arc::clone(&disabled));
    let prepared = reading_submission::prepare_proven(&f.store, &resolver, request.clone(), epoch)
        .await
        .unwrap();
    let prepared = serde_json::to_value(prepared).unwrap();
    assert_eq!(prepared["status"]["type"], "committed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let result = prepared["status"]["result"].clone();
    assert_eq!(
        result["input"]["context"].as_array().unwrap().len(),
        if with_trace { 3 } else { 2 }
    );
    if with_trace {
        let state: (String, Option<String>) = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select status,sent_at from concept_retrieval_traces where id=?1",
                [&trace_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state.0, "sent");
        assert!(state.1.is_some());
        assert_eq!(
            result["input"]["context"][2]["sourceId"],
            format!("retrieval:{trace_id}")
        );
    }
    if queue.is_some() {
        assert!(
            result["input"]["context"]
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["text"]
                    .as_str()
                    .unwrap()
                    .contains("Queued current evidence Ω"))
        );
    }
    disabled.store(true, Ordering::SeqCst);
    let replayed = reading_submission::prepare_proven(&f.store, &resolver, request.clone(), epoch)
        .await
        .unwrap();
    let replayed = serde_json::to_value(replayed).unwrap();
    assert_eq!(replayed["status"]["result"], result);
    assert_eq!(replayed["status"]["proof"]["replayed"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let status =
        reading_submission::prepare_status::read_with_epoch(&f.store, request.clone(), epoch)
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_value(status).unwrap()["status"]["result"],
        result
    );
    let receipt = reading_submission::Receipt {
        conversation_id: conversation,
        request_id: request.request_id,
        paper_id: paper.clone(),
        paper_context_snapshot_id: snapshot.snapshot_id.clone(),
    };
    let record = reading_submission::restore(&f.store, &receipt)
        .await
        .unwrap();
    assert_eq!(
        record.turn_id.to_string(),
        result["turnId"].as_str().unwrap()
    );
    let source_request =
        json!({"conversationId":conversation,"paperId":paper,"turnId":record.turn_id});
    let sources = crate::agent::reading_sources::read(
        &f.store,
        serde_json::from_value(source_request).unwrap(),
    )
    .await
    .unwrap();
    let sources = serde_json::to_value(sources).unwrap();
    assert_eq!(
        sources["retrievalTraceId"],
        if with_trace {
            json!(trace_id)
        } else {
            json!(null)
        }
    );
    assert_eq!(sources["sources"].as_array().unwrap().len(), 3);
    drop(resolver);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
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
    f.bind_actual_package(catalog, key);
    if let Some(queue) = &queue {
        queue.after_reopen(&f, &paper).await;
    }
    let status =
        reading_submission::prepare_status::read_with_epoch(&f.store, request.clone(), epoch)
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_value(status).unwrap()["status"]["result"],
        result
    );
    if generated {
        let restored_preview =
            crate::runtime_plugins::literature_retrieval::commands::read_selected(
                f.store.clone(),
                trace_id.clone(),
            )
            .await
            .unwrap();
        assert_eq!(restored_preview.trace_id, trace_id);
        assert_eq!(
            super::retrieval_preview::context(&f, &trace_id),
            trace_context
        );
    }
    let resolver = models(&f.store, Arc::clone(&calls), Arc::clone(&disabled));
    let replay = reading_submission::prepare_proven(&f.store, &resolver, request, epoch)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap()["status"]["result"],
        result
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        reading_submission::restore(&f.store, &receipt)
            .await
            .unwrap(),
        record
    );
    drop(resolver);
    disabled.store(false, Ordering::SeqCst);
    let wire = Arc::new(ReadingWire {
        source_id: if with_trace {
            record.input.context.last().unwrap().source_id.clone()
        } else {
            record.input.context[0].source_id.clone()
        },
        requests: Default::default(),
    });
    let resolver = models_with_wire(&f.store, Arc::clone(&calls), disabled, wire.clone());
    let sessions =
        Arc::new(crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap());
    let start = crate::agent::reading_start::StartRequest {
        conversation_id: conversation,
        request_id: record.request_id,
        paper_id: paper.clone(),
        paper_context_snapshot_id: snapshot.snapshot_id,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: crate::agent::execution_profile::execution_mode_for_model(
            &record.input.model,
            record.input.reasoning_effort.as_deref(),
        ),
    };
    let admission = crate::agent::reading_start::start_with_epoch(
        &f.store,
        &resolver,
        sessions.clone(),
        start,
        epoch,
    )
    .await
    .unwrap();
    let crate::agent::service::SubmissionAdmission::Started(owned) = admission else {
        panic!("new proven paper submission must start once");
    };
    if let Some(queue) = &queue {
        queue.after_start(&f, &paper).await;
    }
    let completion = tokio::time::timeout(std::time::Duration::from_secs(20), owned.task.wait())
        .await
        .unwrap();
    assert_eq!(
        completion,
        crate::agent::tasks::TaskCompletion::Completed,
        "turn={:?} requests={}",
        sessions
            .open(research_agent_core::conversation::ConversationId::from_uuid(conversation))
            .await
            .unwrap()
            .turn(record.turn_id),
        wire.requests.lock().unwrap().len()
    );
    let requests = wire.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "research_read_source")
    );
    let output = requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    let source: serde_json::Value =
        serde_json::from_str(output["output"].as_str().unwrap()).unwrap();
    assert_eq!(source["sourceId"], wire.source_id);
    if with_trace {
        assert_eq!(source["text"], trace_context);
    }
    assert_eq!(source["turnId"], record.turn_id.to_string());
    assert!(!source["text"].as_str().unwrap().is_empty());
    sessions.shutdown().await.unwrap();
    f.service.shutdown().await.unwrap();
}

/// Only provider transport is synthetic. Core assembles and executes the frozen
/// submission, source tools, durable task and Responses decoder unchanged.
struct ReadingWire {
    source_id: String,
    requests: std::sync::Mutex<Vec<serde_json::Value>>,
}
impl PreviewResponsesWire for ReadingWire {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let mut requests = self.requests.lock().unwrap();
        requests.push(body);
        let index = requests.len();
        let item = match index {
            1 => json!({"id":"source-call","type":"function_call","call_id":"source-call",
                "name":"research_read_source","arguments":json!({"sourceId":self.source_id}).to_string()}),
            2 => json!({"id":"answer","type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"Frozen reading source verified."}]}),
            _ => panic!("unexpected model request {index}"),
        };
        let body: String = [
            json!({"type":"response.created","response":{"id":format!("reading-{index}"),"model":"gpt-5.6-terra"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"id":format!("reading-{index}"),"model":"gpt-5.6-terra","status":"completed","output":[item]}}),
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
