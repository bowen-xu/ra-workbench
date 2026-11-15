//! Real Core run admission and candidate publication use the selected domain package.
use super::*;
use crate::ai::generation_support::{AiRun, AiTaskSpecDraft};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::collections::BTreeMap;

pub(super) async fn evidence(
    f: &Fixture,
    key: &str,
    run: &AiRun,
) -> crate::ai::candidate_contracts::AiEvidenceBundle {
    let prepared = {
        let db = f.store.connection().unwrap();
        crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(&db, "paper", &["selected".into()]).unwrap()
    };
    f.service
        .persist_analysis_docling_evidence(
            f.store.clone(),
            key.into(),
            run.clone(),
            prepared.context_items,
            BTreeMap::new(),
        )
        .await
        .unwrap()
}
pub(super) fn content(
    run: &AiRun,
    bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
) -> String {
    let profile = run.overview_profile.as_ref().unwrap();
    let quote = &bundle.items[0];
    json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":profile.paper_template,"fields":[{"field":profile.field_kinds[0],"status":"filled","statements":[{"id":"question-1","text":"Synthetic research question.","epistemicRole":"source_paraphrase","evidenceItemIds":[quote.citation_key],"evidenceQuotes":[{"evidenceItemId":quote.citation_key,"quote":quote.quote_exact}]}]}]}).to_string()
}
fn counts(f: &Fixture) -> (i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from ai_candidates),(select count(*) from ai_candidate_revisions)",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_candidate_preserves_original_profile_quotes_canonical_bytes_replay_and_rollback()
 {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, input) = analysis_overview::seeded_task(&f);
    f.bind_actual_package(catalog, key.clone());
    let mut task: AiTaskSpecDraft = f
        .service
        .build_analysis_overview_task(&key, input)
        .await
        .unwrap();
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, task.clone())
        .await
        .unwrap();
    task.confirmed_scope_hash = Some(preview.scope_hash);
    task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = f
        .service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            key.clone(),
            task.clone(),
            None,
        )
        .await
        .unwrap();
    let bundle = evidence(&f, &key, &run).await;
    let text = content(&run, &bundle);
    let expected =
        prepared_candidate(&f.store.connection().unwrap(), &run.id, &bundle, &text).unwrap();
    assert!(f.store.generation_policy_source().is_none());
    let saved = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key.clone(),
            run.id.clone(),
            bundle.clone(),
            text.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        saved.payload_hash(),
        crate::ai::generation_support::hash_text(&expected.canonical_json)
    );
    let bytes: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select payload_json from ai_candidate_revisions where candidate_id=?1 and revision=1",
            [&saved.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes, expected.canonical_json);
    assert_eq!(counts(&f), (1, 1));
    let replay = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key.clone(),
            run.id.clone(),
            bundle.clone(),
            format!("```json\n{text}\n```"),
        )
        .await
        .unwrap();
    assert_eq!(replay.id, saved.id);
    assert_eq!(counts(&f), (1, 1));
    let missing = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key.clone(),
            "missing-run".into(),
            bundle.clone(),
            "not JSON".into(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "invalid_input");
    assert!(
        missing
            .to_string()
            .contains("provider 返回的候选不是受支持的结构化 JSON")
    );
    let next = f
        .service
        .admit_analysis_overview(f.store.clone(), credentials, key.clone(), task, None)
        .await
        .unwrap();
    let next_bundle = evidence(&f, &key, &next).await;
    let next_text = content(&next, &next_bundle);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "ai_candidate_revisions"
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key.clone(),
            next.id.clone(),
            next_bundle.clone(),
            next_text.clone(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(counts(&f), (1, 1));
    let recovered = f
        .service
        .save_analysis_candidate(
            f.store.clone(),
            key,
            next.id.clone(),
            next_bundle,
            next_text,
        )
        .await
        .unwrap();
    assert_ne!(recovered.id, saved.id);
    assert_eq!(counts(&f), (2, 2));
    let binding: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select candidate_id from agent_legacy_ai_run_bindings where ai_run_id=?1",
            [next.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(binding, recovered.id);
    f.service.shutdown().await.unwrap();
}
