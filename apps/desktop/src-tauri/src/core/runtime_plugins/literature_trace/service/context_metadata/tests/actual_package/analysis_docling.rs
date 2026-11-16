//! Pure Docling rules use owned original Core context through the real package.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::docling::*, candidate_contracts::EvidenceQuote,
    generation::provider::StoredImageAsset,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_docling_projections_preserve_original_rows_prompt_images_and_rejection() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    paper_preparation::seed(&f);
    let prepared = crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap();
    let items = prepared.context_items;
    let mut evidence = Vec::new();
    let mut assets = BTreeMap::new();
    for item in &items {
        if let Some(row) = protocol::docling_row("run", item).unwrap() {
            evidence.push(EvidenceQuote {
                citation_key: item.id.clone(),
                quote_exact: row.quote,
            });
        }
        if item.kind == "picture" {
            assets.insert(
                item.id.clone(),
                StoredImageAsset {
                    context_item_id: item.id.clone(),
                    content_sha256: "b".repeat(64),
                    data_url: "data:image/png;base64,AAAA".into(),
                },
            );
        }
    }
    let mut prompt = DoclingPromptInputV1 {
        items: items.clone(),
        assets,
        evidence,
        profile: Some(
            serde_json::from_value(json!({
                "paperId":"paper", "selectionRevision":1,
                "profileRefs":[{"profileId":"generic_overview","profileVersion":1}],
                "fieldKinds":["research_question"]
            }))
            .unwrap(),
        ),
    };
    let expected = protocol::docling_prompt(&prompt).unwrap();
    assert!(expected.has_images());
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    let changes = f.store.connection().unwrap().total_changes();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "bibliographic_items"
                        | "docling_parse_runs"
                        | "ai_runs"
                        | "ai_scope_items"
                        | "ai_evidence_bundles"
                        | "ai_evidence_items"
                        | "ai_docling_evidence_context_items",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    for item in items {
        let expected = protocol::docling_row("run", &item).unwrap();
        let (actual, origin) = f
            .service
            .project_docling_evidence_row(
                &key,
                DoclingRowInputV1 {
                    run_id: "run".into(),
                    item,
                },
            )
            .await
            .unwrap();
        let actual = match actual {
            DoclingRowDecisionV1::Ready { row } => Some(row),
            DoclingRowDecisionV1::Skipped {} => None,
            DoclingRowDecisionV1::Unavailable {} => panic!("original row projection failed"),
        };
        assert_eq!(actual, expected);
        assert_eq!(origin.package_key, key);
        assert!(!origin.native_request_id.is_empty());
    }
    let (actual, origin) = f
        .service
        .render_docling_overview_prompt(&key, prompt.clone())
        .await
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(origin.package_key, key);
    assert!(!origin.native_request_id.is_empty());
    prompt.profile = None;
    let failed = f
        .service
        .render_docling_overview_prompt(&key, prompt)
        .await
        .unwrap_err();
    assert_eq!(failed.code(), "invalid_input");
    assert!(
        failed
            .to_string()
            .contains("结构化阅读 AI 提取缺少已确认阅读框架")
    );
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(f.store.connection().unwrap().total_changes(), changes);
    f.service.shutdown().await.unwrap();
}
