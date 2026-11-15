//! Original selected-text entry against the independent package and Core admission.
use super::*;
use crate::ai::{JsonCredentialStore, generation_support::AiTaskSpecDraft};

fn task(f: &Fixture) -> (JsonCredentialStore, AiTaskSpecDraft) {
    let (credentials, mut draft) = analysis_overview::suggestion_task(f, "concept_suggestion_v1");
    let db = f.store.connection().unwrap();
    db.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values('reference-anchor','paper','selected',?1,0,'1','[]','Doe (2024). Grounded research systems.','Context before. ',' After Ω.','now')",["a".repeat(64)]).unwrap();
    draft.task_kind = "reference_extraction".into();
    draft.suggestion_candidate_kind = None;
    draft.scope_items = vec![
        research_workbench_native_sdk::generation::AiScopeItemDraft {
            item_kind: "anchor".into(),
            object_id: "reference-anchor".into(),
            parent_object_id: None,
            content_sha256: None,
            object_revision: None,
        },
    ];
    draft.user_goal = "Extract the supplied reference fields for review.".into();
    (credentials, draft)
}
fn stable(preview: &crate::ai::generation_support::AiTaskPreview) -> Value {
    let mut value = serde_json::to_value(preview).unwrap();
    for item in value["scopeItems"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("id");
        item.as_object_mut().unwrap().remove("createdAt");
    }
    value
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_preview_and_admission_preserve_original_scope_and_two_items() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = task(&f);
    let expected = prepared_preview(&f.store.connection().unwrap(),
        &credentials,
        &draft,
    )
    .unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
        .await
        .unwrap();
    assert_eq!(stable(&preview), stable(&expected));
    assert_eq!(preview.evidence_preview.len(), 2);
    assert_eq!(preview.evidence_preview[0].citation_key, "ev-quote");
    assert_eq!(
        preview.evidence_preview[1].context_text,
        "Context before. Doe (2024). Grounded research systems. After Ω."
    );
    draft.confirmed_scope_hash = Some(preview.scope_hash.clone());
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = f
        .service
        .admit_analysis_overview(f.store.clone(), credentials, key, draft, None)
        .await
        .unwrap();
    assert_eq!(run.scope_hash, preview.scope_hash);
    assert_eq!(run.status, "queued");
    assert!(run.shared_conversation_id.is_some());
    assert!(run.shared_request_id.is_some());
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_preview_preserves_original_unicode_budgets_and_errors() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, draft) = task(&f);
    f.bind_actual_package(catalog, key.clone());
    for (quote, prefix) in [
        (" ".into(), String::new()),
        ("Ω".repeat(1501), String::new()),
        ("Ω".repeat(1500), "Ω".repeat(10501)),
        ("Ω".repeat(1500), "Ω".repeat(10491)),
    ] {
        f.store.connection().unwrap().execute("update source_anchors set quote_exact=?1,quote_prefix=?2 where id='reference-anchor'",rusqlite::params![quote,prefix]).unwrap();
        let expected = prepared_preview(&f.store.connection().unwrap(),
            &credentials,
            &draft,
        );
        let actual = f
            .service
            .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
            .await;
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(stable(&actual), stable(&expected)),
            (Err(expected), Err(actual)) => assert!(
                actual.to_string().contains(&expected.to_string()),
                "expected={expected} actual={actual}"
            ),
            _ => panic!("static and Native preview disagree"),
        }
    }
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from ai_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[cfg(all(debug_assertions, feature = "development-data"))]
mod execution;
