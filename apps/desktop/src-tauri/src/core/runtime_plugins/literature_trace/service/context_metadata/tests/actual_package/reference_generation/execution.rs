//! Actual shared Agent/Native generation with original Core evidence and candidate owners.
use super::super::task_generation_host as host;
use super::*;
use crate::ai::{
    CancellationSignal,
    generation_support::{AiRun, execute_research_task},
};
use std::sync::atomic::Ordering;
mod acceptance;
mod review;
mod revision;
mod storage;

fn setup() -> (Fixture, String, JsonCredentialStore, AiTaskSpecDraft) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = task(&f);
    let selection = draft.model_selection.as_mut().unwrap();
    selection.provider_id = "openai".into();
    selection.model_id = "gpt-5.6-terra".into();
    draft.user_goal="Research question. Do not invoke any tool. Extract the reference fields from the supplied evidence.".into();
    f.store.connection().unwrap().execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'https://api.openai.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    (f, key, credentials, draft)
}
async fn admit(
    f: &Fixture,
    key: &str,
    credentials: &JsonCredentialStore,
    mut draft: AiTaskSpecDraft,
) -> AiRun {
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), key, draft.clone())
        .await
        .unwrap();
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    f.service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            key.into(),
            draft,
            None,
        )
        .await
        .unwrap()
}
fn payload() -> Value {
    json!({"kind":"reference_record_candidate_v1","schemaVersion":1,"rawText":"Doe (2024). Grounded research systems.","evidenceItemIds":["ev-quote"],"fields":[{"name":"title","value":"Grounded research systems","evidenceItemIds":["ev-quote"]}],"uncertainties":["Identity and DOI remain unverified."]})
}
fn snapshot(f: &Fixture, run: &str) -> Value {
    let db = f.store.connection().unwrap();
    let mut value = serde_json::Map::new();
    for (table, column) in [
        ("ai_runs", "id"),
        ("ai_scope_items", "run_id"),
        ("ai_run_stages", "run_id"),
        ("ai_evidence_bundles", "run_id"),
        ("ai_candidates", "run_id"),
        ("agent_legacy_ai_run_bindings", "ai_run_id"),
    ] {
        let mut query = db
            .prepare(&format!(
                "select * from {table} where {column}=?1 order by rowid"
            ))
            .unwrap();
        let columns = query.column_count();
        let rows = query
            .query_map([run], |row| {
                (0..columns)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .map(|row| format!("{:?}", row.unwrap()))
            .collect::<Vec<_>>();
        value.insert(table.into(), json!(rows));
    }
    Value::Object(value)
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_generation_preserves_agent_candidate_and_normal_reopen() {
    let (mut f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let signal = CancellationSignal::default();
    let text = payload();
    let (host, wire) = host::Host::new(&f, credentials, run.clone(), signal.clone(), text.clone());
    execute_research_task(&host, &run, &signal).await.unwrap();
    assert_eq!(wire.calls.load(Ordering::SeqCst), 1);
    let bundle = f
        .service
        .persist_selection_task_evidence(f.store.clone(), key.clone(), run.id.clone())
        .await
        .unwrap();
    assert_eq!(bundle.items.len(), 2);
    assert_eq!(
        bundle.items[0].quote_exact,
        "Doe (2024). Grounded research systems."
    );
    assert_eq!(
        bundle.items[1].context_text,
        "Context before. Doe (2024). Grounded research systems. After Ω."
    );
    let expected = prepared_candidate(
        &f.store.connection().unwrap(),
        &run.id,
        &bundle,
        &text.to_string(),
    )
    .unwrap();
    let saved = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key.clone(),
            run.id.clone(),
            bundle.clone(),
            text.to_string(),
        )
        .await
        .unwrap();
    assert_eq!(
        saved.payload_hash(),
        crate::ai::generation_support::hash_text(&expected.canonical_json)
    );
    {
        let db = f.store.connection().unwrap();
        let(status,terminal,count,bytes):(String,String,i64,String)=db.query_row("select r.status,b.terminal_status,(select count(*) from ai_candidates where run_id=r.id),v.payload_json from ai_runs r join agent_legacy_ai_run_bindings b on b.ai_run_id=r.id join ai_candidates c on c.run_id=r.id join ai_candidate_revisions v on v.candidate_id=c.id where r.id=?1",[&run.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(
            (status.as_str(), terminal.as_str(), count),
            ("needs_review", "needs_review", 1)
        );
        assert_eq!(bytes, expected.canonical_json);
        let stages: Vec<String> = db
            .prepare("select status from ai_run_stages where run_id=?1 order by created_at,rowid")
            .unwrap()
            .query_map([&run.id], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(stages, ["succeeded"; 5]);
        crate::ai::review_support::ReviewFacts::new(&db)
            .require_completed_turn(&saved.id)
            .unwrap();
    }
    let candidate_before = serde_json::to_value(
        crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&saved.id)
            .unwrap(),
    )
    .unwrap();
    let before = snapshot(&f, &run.id);
    let frozen = serde_json::to_value(&bundle).unwrap();
    drop(host);
    f = reopen_actual_fixture(f, false).await;
    assert_eq!(snapshot(&f, &run.id), before);
    let candidate_after = serde_json::to_value(
        crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&saved.id)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(candidate_after, candidate_before);
    let replay = f
        .service
        .persist_selection_task_evidence(f.store.clone(), key, run.id.clone())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), frozen);
    crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
        .require_completed_turn(&saved.id)
        .unwrap();
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_generation_retains_provider_failure_and_cancel() {
    for canceled in [false, true] {
        let (f, key, credentials, draft) = setup();
        let run = admit(&f, &key, &credentials, draft).await;
        let signal = CancellationSignal::default();
        let (host, wire) = host::Host::new(&f, credentials, run.clone(), signal.clone(), payload());
        let error = if canceled {
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            *wire.pause.lock().unwrap() = Some((started.clone(), release.clone()));
            let work = execute_research_task(&host, &run, &signal);
            let cancel = async {
                started.notified().await;
                let store = f.store.clone();
                assert!(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        tokio::task::spawn_blocking(move || store.connection().is_ok())
                    )
                    .await
                    .unwrap()
                    .unwrap()
                );
                signal.cancel();
                release.notify_one();
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                tokio::join!(work, cancel)
            })
            .await
            .unwrap();
            result.unwrap_err()
        } else {
            wire.status.store(429, Ordering::SeqCst);
            execute_research_task(&host, &run, &signal)
                .await
                .unwrap_err()
        };
        assert_eq!(
            error.code(),
            if canceled { "canceled" } else { "rate_limited" }
        );
        assert!(!error.to_string().contains("synthetic private provider"));
        host.terminal_failure(&error).await.unwrap();
        let db = f.store.connection().unwrap();
        let(status,count):(String,i64)=db.query_row("select status,(select count(*) from ai_candidates where run_id=r.id) from ai_runs r where id=?1",[&run.id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(
            (status.as_str(), count),
            (if canceled { "canceled" } else { "failed" }, 0)
        );
        drop(db);
        drop(host);
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_review_decisions_use_core_owned_candidate_facts() {
    use research_workbench_native_sdk::{
        analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
        candidate_review::{contracts::AiReviewDecisionDraft, wire::ReviewInputV1},
    };
    let (f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let signal = CancellationSignal::default();
    let (host, _) = host::Host::new(&f, credentials, run.clone(), signal.clone(), payload());
    execute_research_task(&host, &run, &signal).await.unwrap();
    let candidate = {
        let db = f.store.connection().unwrap();
        let id: String = db
            .query_row(
                "select id from ai_candidates where run_id=?1",
                [&run.id],
                |row| row.get(0),
            )
            .unwrap();
        crate::ai::review_support::ReviewFacts::new(&db)
            .candidate(&id)
            .unwrap()
    };
    let draft = AiReviewDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        action: "accept_unchanged".into(),
        rationale: "Reviewed synthetic source.".into(),
        idempotency_key: "native-review-decision".into(),
        reading_unit_title: None,
        project_id: None,
    };
    for input in [
        ReviewInputV1::ValidateDraft {
            draft: draft.clone(),
        },
        ReviewInputV1::Candidate {
            draft: draft.clone(),
            observation: candidate.review_observation(draft.candidate_revision),
        },
        ReviewInputV1::Inspection { inspected: false },
        ReviewInputV1::ProjectMembership { contains: false },
        ReviewInputV1::Replay {
            draft: draft.clone(),
            existing: (
                candidate.id.clone(),
                candidate.current_revision,
                draft.action.clone(),
                draft.rationale.clone(),
            ),
        },
    ] {
        let expected = research_literature_library_native::candidate_review::decide(input.clone());
        let (decision, origin) = f
            .service
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::CandidateReview {
                    input: Box::new(input.clone()),
                },
            )
            .await
            .unwrap();
        let WorkflowDecisionV1::CandidateReview { decision } = decision else {
            panic!("wrong family")
        };
        assert!(input.matches_decision(&decision));
        assert_eq!(decision, expected);
        assert!(!origin.instance_id.is_empty());
        assert!(!origin.native_request_id.is_empty());
        assert_eq!(origin.package_key, key);
    }
    assert!(f.store.generation_policy_source().is_none());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from ai_review_decisions where candidate_id=?1",
                [&candidate.id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(host);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
