//! Finite per-item rules execute in the actual independent package.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    candidate_review::items::{
        AiSuggestionItemDecisionDraft, ItemReviewDecisionV1, ItemReviewInputV1,
    },
};
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_item_rules_preserve_hash_and_original_rejection() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key.clone());
    let draft = AiSuggestionItemDecisionDraft {
        candidate_id: "historical".into(),
        candidate_revision: 1,
        suggestion_id: "concept-1".into(),
        action: "defer".into(),
        rationale: "Checked synthetic Ω".into(),
        idempotency_key: "item-decision".into(),
        resolution: None,
    };
    let expected = protocol::request_hash(&draft).unwrap();
    let (decision, origin) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::SuggestionReview {
                input: Box::new(ItemReviewInputV1::Draft {
                    draft: draft.clone(),
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        decision,
        WorkflowDecisionV1::SuggestionReview {
            decision: ItemReviewDecisionV1::Draft {
                result: Ok(expected)
            }
        }
    );
    assert_eq!(origin.package_key, key);
    let mut invalid = draft;
    invalid.candidate_revision = 0;
    let (decision, _) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::SuggestionReview {
                input: Box::new(ItemReviewInputV1::Draft { draft: invalid }),
            },
        )
        .await
        .unwrap();
    let WorkflowDecisionV1::SuggestionReview {
        decision: ItemReviewDecisionV1::Draft { result: Err(error) },
    } = decision
    else {
        panic!("original input rejection");
    };
    assert!(error.to_string().contains("建议条目审查参数无效"));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from ai_suggestion_item_decisions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[cfg(all(debug_assertions, feature = "development-data"))]
mod execution;
