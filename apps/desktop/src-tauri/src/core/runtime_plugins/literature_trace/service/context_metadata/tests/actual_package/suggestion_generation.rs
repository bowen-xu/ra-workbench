//! Retained suggestion entry, actual Native policy and original shared Agent/storage.
use super::*;
use crate::ai::{
    CancellationSignal, JsonCredentialStore,
    generation_support::{AiRun, AiTaskSpecDraft, execute_suggestions},
};
use std::sync::atomic::Ordering;
mod evidence;
use super::task_generation_host as host;

pub(super) fn setup() -> (Fixture, String, JsonCredentialStore, AiTaskSpecDraft) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = analysis_overview::suggestion_task(&f, "concept_suggestion_v1");
    let selection = draft.model_selection.as_mut().unwrap();
    selection.provider_id = "openai".into();
    selection.model_id = "gpt-5.6-terra".into();
    f.store.connection().unwrap().execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'https://api.openai.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.paper_generation_source().is_none());
    assert!(f.store.generation_policy_source().is_none());
    (f, key, credentials, draft)
}

pub(super) async fn admit(
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

pub(super) fn payload(kind: &str) -> Value {
    let item = match kind {
        "concept_suggestion_v1" => {
            json!({"id":"concept_1","term":"稀疏门控","explanation":"每次路由只激活部分专家。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]})
        }
        "definition_suggestion_v1" => {
            json!({"id":"definition_1","term":"稀疏门控","statement":"稀疏门控在一次计算中选择部分专家。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]})
        }
        "relation_suggestion_v1" => {
            json!({"id":"relation_1","subjectTerm":"稀疏门控","predicateKind":"related_to","objectTerm":"混合专家","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]})
        }
        "attention_suggestion_v1" => {
            json!({"id":"attention_1","title":"关注负载均衡","reason":"路由约束可能影响专家利用率。","priority":"high","evidenceItemIds":["ev-page-1"]})
        }
        _ => panic!("unknown fixture kind"),
    };
    json!({"kind":kind,"schemaVersion":1,"suggestions":[item]})
}

fn stages(f: &Fixture, run: &str) -> Vec<String> {
    f.store
        .connection()
        .unwrap()
        .prepare("select status from ai_run_stages where run_id=?1 order by created_at,rowid")
        .unwrap()
        .query_map([run], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

pub(super) fn assert_completed(f: &Fixture, run: &str) {
    let db = f.store.connection().unwrap();
    let (conversation, turn): (String, String) = db.query_row(
        "select b.conversation_id,s.turn_id from agent_legacy_ai_run_bindings b join agent_submissions s on s.conversation_id=b.conversation_id and s.request_id=b.request_id where b.ai_run_id=?1",
        [run], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    let outcome = crate::agent::repository::projection::read_turn_outcome(
        &db,
        research_agent_core::conversation::ConversationId::from_uuid(
            uuid::Uuid::parse_str(&conversation).unwrap(),
        ),
        uuid::Uuid::parse_str(&turn).unwrap(),
    )
    .unwrap();
    assert_eq!(
        outcome,
        Some(research_agent_core::protocol::TurnOutcome::Completed)
    );
}

fn snapshot(f: &Fixture, run: &str) -> Value {
    let db = f.store.connection().unwrap();
    let mut snapshot = serde_json::Map::new();
    // Full original row bytes, including durable identities and timestamps.
    for (table, column) in [
        ("ai_runs", "id"),
        ("ai_scope_items", "run_id"),
        ("ai_run_stages", "run_id"),
        ("ai_suggestion_run_contexts", "run_id"),
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
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .map(|row| format!("{:?}", row.unwrap()))
            .collect::<Vec<_>>();
        snapshot.insert(table.into(), json!(rows));
    }
    Value::Object(snapshot)
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_generation_keeps_four_kinds_evidence_candidate_agent_and_reopen()
{
    let (mut f, key, credentials, draft) = setup();
    let mut saved = Vec::new();
    for kind in research_workbench_native_sdk::generation::SUGGESTION_CANDIDATE_KINDS {
        let mut task = draft.clone();
        task.suggestion_candidate_kind = Some(kind.into());
        let run = admit(&f, &key, &credentials, task).await;
        let signal = CancellationSignal::default();
        let text = payload(kind);
        let (host, wire) = host::Host::new(
            &f,
            credentials.clone(),
            run.clone(),
            signal.clone(),
            text.clone(),
        );
        execute_suggestions(&host, &run, &signal).await.unwrap();
        assert_eq!(wire.calls.load(Ordering::SeqCst), 1);
        assert_eq!(stages(&f, &run.id), ["succeeded"; 5]);
        let bundle = f
            .service
            .persist_page_task_evidence(f.store.clone(), key.clone(), run.id.clone())
            .await
            .unwrap();
        assert_eq!(bundle.items.len(), 1);
        assert_eq!(bundle.items[0].citation_key, "ev-page-1");
        assert_eq!(
            bundle.items[0].quote_exact,
            "Research question. Methods. Results. Synthetic page Ω"
        );
        let expected = prepared_candidate(
            &f.store.connection().unwrap(),
            &run.id,
            &bundle,
            &text.to_string(),
        )
        .unwrap();
        let candidate = f
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
            candidate.payload_hash(),
            crate::ai::generation_support::hash_text(&expected.canonical_json)
        );
        let db = f.store.connection().unwrap();
        let (status, terminal, count, bytes): (String, String, i64, String) = db.query_row(
            "select r.status,b.terminal_status,(select count(*) from ai_candidates where run_id=r.id),v.payload_json from ai_runs r join agent_legacy_ai_run_bindings b on b.ai_run_id=r.id join ai_candidates c on c.run_id=r.id join ai_candidate_revisions v on v.candidate_id=c.id where r.id=?1",
            [&run.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(
            (status.as_str(), terminal.as_str(), count),
            ("needs_review", "needs_review", 1)
        );
        assert_eq!(bytes, expected.canonical_json);
        drop(db);
        assert_completed(&f, &run.id);
        drop(host);
        saved.push((
            run.id.clone(),
            snapshot(&f, &run.id),
            serde_json::to_value(bundle).unwrap(),
        ));
    }
    f = reopen_actual_fixture(f, false).await;
    for (run, before, bundle) in saved {
        assert_eq!(snapshot(&f, &run), before);
        assert_completed(&f, &run);
        let after = f
            .service
            .persist_page_task_evidence(f.store.clone(), key.clone(), run)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), bundle);
    }
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_generation_provider_failure_and_cancel_keep_original_terminals()
{
    let (f, key, credentials, draft) = setup();
    for canceled in [false, true] {
        let run = admit(&f, &key, &credentials, draft.clone()).await;
        let signal = CancellationSignal::default();
        let (host, wire) = host::Host::new(
            &f,
            credentials.clone(),
            run.clone(),
            signal.clone(),
            payload("concept_suggestion_v1"),
        );
        let error = if canceled {
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            *wire.pause.lock().unwrap() = Some((started.clone(), release.clone()));
            let work = execute_suggestions(&host, &run, &signal);
            let cancel = async {
                started.notified().await;
                let store = f.store.clone();
                let available = tokio::task::spawn_blocking(move || store.connection().is_ok());
                assert!(
                    tokio::time::timeout(std::time::Duration::from_secs(2), available)
                        .await
                        .unwrap()
                        .unwrap(),
                    "model wait holds no DB guard"
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
            execute_suggestions(&host, &run, &signal).await.unwrap_err()
        };
        let terminal = if canceled { "canceled" } else { "failed" };
        assert!(
            !error
                .to_string()
                .contains("synthetic private provider failure")
        );
        host.terminal_failure(&error).await.unwrap();
        assert_eq!(
            stages(&f, &run.id),
            ["succeeded", "succeeded", "succeeded", "failed"]
        );
        let state: (String, String, i64) = f.store.connection().unwrap().query_row(
            "select r.status,b.terminal_status,(select count(*) from ai_candidates where run_id=r.id) from ai_runs r join agent_legacy_ai_run_bindings b on b.ai_run_id=r.id where r.id=?1", [&run.id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(
            (state.0.as_str(), state.1.as_str(), state.2),
            (terminal, terminal, 0)
        );
    }
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
