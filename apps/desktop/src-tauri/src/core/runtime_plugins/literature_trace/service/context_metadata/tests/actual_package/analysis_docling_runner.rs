//! The original overview entry runs the selected package and real Core model decoder.
use super::*;
use crate::{
    agent::{
        model::{
            AgentHost, AgentModelResolver, PreparedAgentModel,
            responses_transport::PreviewResponsesWire,
        },
        service::AgentSessionService,
    },
    ai::{
        CancellationSignal, analysis_support::AnalysisExecution,
        paper_analysis_contracts::DoclingPaperOverviewDraft,
        paper_reading_contracts::PaperAiReadingImageAssetDraft,
    },
};
use research_agent_core::{
    model::{ModelCapabilities, ModelFuture},
    protocol::ModelIdentity,
};
use std::sync::Mutex;
use uuid::Uuid;

pub(super) struct Wire {
    pub(super) text: Mutex<String>,
    pub(super) calls: Mutex<usize>,
    step: Mutex<usize>,
    path: String,
    pub(super) pause: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}
impl PreviewResponsesWire for Wire {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        let input: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert!(
            input
                .to_string()
                .contains("Final chat text is not a write receipt.")
        );
        assert!(
            input["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "research_apply_patch")
        );
        *self.calls.lock().unwrap() += 1;
        let text = self.text.lock().unwrap().clone();
        if text == "provider rejected" {
            return Box::pin(async {
                Ok(hyper::Response::builder()
                    .status(429)
                    .body(reqwest::Body::from(
                        "synthetic provider body must remain private",
                    ))
                    .unwrap()
                    .into())
            });
        }
        let mut step = self.step.lock().unwrap();
        let item = {
            let item = match *step {
                0 => {
                    json!({"id":"read-target","type":"function_call","call_id":"read-target","name":"research_read_file","arguments":json!({"path":self.path}).to_string()})
                }
                1 => {
                    let read = input["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .rev()
                        .find(|item| {
                            item["type"] == "function_call_output"
                                && item["call_id"] == "read-target"
                        })
                        .unwrap();
                    let read: Value =
                        serde_json::from_str(read["output"].as_str().unwrap()).unwrap();
                    let before = read["text"].as_str().unwrap();
                    let mut patch =
                        format!("*** Begin Patch\n*** Update File: {}\n@@\n", self.path);
                    for line in before.lines() {
                        patch.push_str(&format!("-{line}\n"));
                    }
                    patch.push_str(&format!("+{text}\n*** End Patch"));
                    json!({"id":"write-target","type":"function_call","call_id":"write-target","name":"research_apply_patch","arguments":json!({"patch":patch}).to_string()})
                }
                2 => {
                    assert!(input.to_string().contains("applied"));
                    json!({"id":"final", "type":"message", "role":"assistant", "status":"completed", "content":[{"type":"output_text", "text":"Synthetic saved result."}]})
                }
                _ => panic!("unexpected synthetic model step"),
            };
            *step = (*step + 1) % 3;
            item
        };
        let body: String = [
            json!({"type":"response.created","response":{"id":"docling","model":"gpt-5.6-terra"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"id":"docling","model":"gpt-5.6-terra","status":"completed","output":[item],"usage":{"input_tokens":11,"output_tokens":7}}}),
        ].into_iter().map(|event| format!("data: {event}\n\n")).collect();
        let pause = self.pause.lock().unwrap().take();
        Box::pin(async move {
            if let Some((started, release)) = pause {
                started.notify_one();
                release.notified().await;
            }
            Ok(hyper::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::from(body))
                .unwrap()
                .into())
        })
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_docling_runner_preserves_success_snapshot_reuse_rejection_and_cancel() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut input) = analysis_overview::seeded_task(&f);
    let selection = input.model_selection.as_mut().unwrap();
    selection.provider_id = "openai".into();
    selection.model_id = "gpt-5.6-terra".into();
    f.store.connection().unwrap().execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'https://api.openai.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
    let prepared = crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap();
    let profile = prepared_template(
        &f.store.connection().unwrap(),
        "paper",
        input.template.as_ref().unwrap(),
    )
    .unwrap();
    let payload = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":profile.paper_template,"fields":[{"field":"question","status":"not_found","statements":[]}]}).to_string();
    let image = PaperAiReadingImageAssetDraft {
        context_item_id: prepared.image_requests[0].context_item_id.clone(),
        data_url: "data:image/png;base64,iVBORw0KGgpzeW50aGV0aWM=".into(),
    };
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key);
    assert!(f.store.generation_policy_source().is_none());
    assert!(f.store.paper_reading_source().is_none());
    let (execution, wire) = setup_execution(&f, credentials, payload);
    let mut draft = DoclingPaperOverviewDraft {
        paper_id: input.paper_id,
        attachment_ids: input.attachment_ids,
        context_snapshot_id: None,
        overview_profile_selection_revision: input.selection_revision,
        paper_template_ref: input.template,
        model_selection: input.model_selection,
        image_assets: vec![],
        shared_conversation_id: None,
        shared_request_id: None,
    };
    let missing = execution
        .overview(draft.clone(), CancellationSignal::default())
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "invalid_input");
    assert!(missing.message().contains("图片截图数量"));
    assert_eq!(*wire.calls.lock().unwrap(), 0);
    draft.image_assets = vec![image];
    let first = execution
        .overview(draft.clone(), CancellationSignal::default())
        .await
        .unwrap();
    let (snapshot, status, candidates): (String,String,i64) = f.store.connection().unwrap().query_row("select context_snapshot_id,status,(select count(*) from ai_candidates where run_id=ai_runs.id) from ai_runs where id=?1",[&first],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!((status.as_str(), candidates), ("needs_review", 1));
    assert_stages(&f, &first, &["succeeded", "succeeded", "succeeded"]);
    draft.context_snapshot_id = Some(snapshot.clone());
    draft.image_assets.clear();
    let mut next: Value = serde_json::from_str(&wire.text.lock().unwrap()).unwrap();
    next["fields"][0]["status"] = json!("filled");
    next["fields"][0]["statements"] = json!([{
        "id":"question-2", "text":"Synthetic second question.", "epistemicRole":"source_paraphrase",
        "evidenceItemIds":["P1-T1"], "evidenceQuotes":[{"evidenceItemId":"P1-T1","quote":"Prepared evidence Ω"}]
    }]);
    *wire.text.lock().unwrap() = next.to_string();
    let second = execution
        .overview(draft.clone(), CancellationSignal::default())
        .await
        .unwrap();
    assert_ne!(first, second);
    let reused: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select context_snapshot_id from ai_runs where id=?1",
            [&second],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshot, reused);
    assert_stages(&f, &second, &["succeeded", "succeeded", "succeeded"]);
    *wire.text.lock().unwrap() = "provider rejected".into();
    let rejected = execution
        .overview(draft.clone(), CancellationSignal::default())
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), "rate_limited");
    assert!(!rejected.message().contains("synthetic provider body"));
    let signal = CancellationSignal::default();
    signal.cancel();
    let canceled = execution.overview(draft, signal).await.unwrap_err();
    assert_eq!(canceled.code(), "canceled");
    assert_eq!(
        *wire.calls.lock().unwrap(),
        6 + usize::from(research_agent_core::resilience::RetryPolicy::DEFAULT.max_attempts)
    );
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    let counts: (i64,i64,i64,i64) = db.query_row("select (select count(*) from ai_runs where status='needs_review'),(select count(*) from ai_runs where status='failed'),(select count(*) from ai_runs where status='canceled'),(select count(*) from ai_candidates)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(counts, (2, 1, 1, 2));
    drop(db);
    f.service.shutdown().await.unwrap();
}
fn assert_stages(f: &Fixture, run: &str, expected: &[&str]) {
    let db = f.store.connection().unwrap();
    let mut query = db
        .prepare("select status from ai_run_stages where run_id=?1 order by created_at,rowid")
        .unwrap();
    let actual = query
        .query_map([run], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actual, expected);
}

pub(super) fn setup_execution(
    f: &Fixture,
    credentials: crate::ai::JsonCredentialStore,
    payload: String,
) -> (AnalysisExecution, Arc<Wire>) {
    let path = crate::structured_resources::find_by_paper(
        &f.store.connection().unwrap(),
        "paper",
        "structuredReading",
    )
    .unwrap()
    .source_uri;
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    let wire = Arc::new(Wire {
        text: Mutex::new(payload),
        calls: Mutex::new(0),
        step: Mutex::new(0),
        path,
        pause: Mutex::new(None),
    });
    let model_wire = wire.clone();
    let models = AgentModelResolver::for_preview(
        &f.store,
        Arc::new(move |host, selection| {
            assert!(matches!(host, AgentHost::Review));
            let selection = selection.unwrap();
            assert_eq!(
                (&*selection.provider_id, &*selection.model_id),
                ("openai", "gpt-5.6-terra")
            );
            PreparedAgentModel::for_preview(
                ModelIdentity {
                    provider_id: selection.provider_id,
                    model_id: selection.model_id,
                    revision: None,
                },
                None,
                None,
                ModelCapabilities {
                    vision: true,
                    tools: false,
                    parallel_tools: false,
                    reasoning_efforts: vec![],
                },
                model_wire.clone(),
                Uuid::from_u128(0xd0c11),
            )
        }),
    )
    .unwrap();
    let execution = AnalysisExecution::new(f.store.clone(), credentials, models, sessions);
    (execution, wire)
}
