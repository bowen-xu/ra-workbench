//! Original task renderer over real Core grants and the independently built rules.
use super::*;
use crate::agent::context_session::{
    resource_grants,
    sources::{Access, Resource},
};
use research_workbench_native_sdk::analysis_workflow::{AnalysisWorkflowError, prompt::*};

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_prompt_preserves_core_grants_text_and_composer_bindings() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    paper_preparation::seed(&f);
    let context = {
        let db = f.store.connection().unwrap();
        let reading =
            crate::structured_resources::find_by_paper(&db, "paper", "structuredReading").unwrap();
        resource_grants::grants(
            &db,
            &[
                Resource::Paper {
                    resource_id: "paper".into(),
                    attachment_id: Some("selected".into()),
                    page_index: None,
                },
                Resource::StructuredObject {
                    resource_id: reading.id,
                    access: Some(Access::ReadWrite),
                },
            ],
            None,
        )
        .unwrap()
    };

    let input = TaskPromptInputV1 {
        task_template: include_str!(
            "../../../../../../../../../../../packages/core/prompts/structured-task.md"
        )
        .into(),
        reading_contract: include_str!(
            "../../../../../../../../../../../packages/core/prompts/structured-reading-contract.md"
        )
        .into(),
        sources: context
            .iter()
            .filter_map(|source| {
                crate::ai::structured_generation_support::prompt_resource(source).map(|resource| {
                    PromptSourceV1 {
                        source: source.clone(),
                        resource,
                    }
                })
            })
            .collect(),
        kind: "structuredReading".into(),
        profile: None,
        preferences: "".into(),
    };
    let expected =
        research_literature_library_native::analysis_workflow::task_prompt::render_current(
            &input.task_template,
            &input.reading_contract,
            input
                .sources
                .iter()
                .map(|entry| (&entry.source, entry.resource.clone())),
            &input.kind,
            input.profile.as_ref(),
            &input.preferences,
        )
        .unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    let (result, origin) = f
        .service
        .render_analysis_task_prompt(&key, input.clone())
        .await
        .unwrap();
    let actual = result.unwrap();
    assert_eq!(actual, expected);
    assert_eq!(origin.package_key, key);
    assert!(!origin.native_request_id.is_empty());
    let json = serde_json::to_value(&actual).unwrap();
    assert!(
        json["userInput"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["target"].get("revision").is_none())
    );
    let mut readonly = input;
    readonly
        .sources
        .retain(|entry| matches!(entry.resource, PromptResourceV1::Paper { .. }));
    let (result, _) = f
        .service
        .render_analysis_task_prompt(&key, readonly)
        .await
        .unwrap();
    assert_eq!(
        result,
        Err(AnalysisWorkflowError::InvalidInput(
            "内部任务目标未授予读写权限，请在上下文管理中选择目标并允许读写".into()
        ))
    );
    let count: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from paper_analysis_workflows", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    f.service.shutdown().await.unwrap();
}
