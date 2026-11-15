use super::*;
use crate::agent::composed_tools::RejectingToolExecutor;
use research_agent_core::{model::ToolCall, tool_registry::OwnedToolRegistration};

#[tokio::test]
async fn shared_text_patch_updates_the_leased_copy_reports_mixed_failure_and_preserves_delivery() {
    let f = fixture(true);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(AgentModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let record = prepared.record;
    let copy = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: record.request_id,
                manuscript_id: f.manuscript,
            },
            record.context_snapshot_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    drop(service.approve_workspace(copy).await.unwrap());
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
    };
    let context = record.context_snapshot_id;
    let turn_id = record.turn_id;
    let registered = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        record,
        OwnedToolRegistration::new(vec![], std::sync::Arc::new(RejectingToolExecutor)),
    );
    let (definitions, tools) = registered.into_parts();
    let patch_tool = definitions
        .iter()
        .find(|definition| definition.name == "research_apply_patch")
        .unwrap();
    assert_eq!(
        patch_tool.parameters["required"],
        serde_json::json!(["patch"])
    );
    let path = format!("source://writings/{}/main.tex", f.manuscript);
    let read = tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "read".into(),
            name: "research_read_file".into(),
            arguments: serde_json::json!({"path":path}),
        })
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.content);
    let patch = format!(
        "*** Begin Patch\n*** Update File: {path}\n@@\n-Frozen manuscript\n+Changed manuscript\n*** Add File: source://notes/unregistered\n+---\n+title: Denied root\n+---\n+body\n*** End Patch"
    );
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "mixed-patch".into(),
        name: "research_apply_patch".into(),
        arguments: serde_json::json!({"patch":patch}),
    };
    let output = tools.authorize_and_execute(call.clone()).await.unwrap();
    let result: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["targets"][0]["status"], "applied");
    assert_eq!(result["targets"][1]["status"], "failed");
    assert_eq!(
        output.content,
        tools.authorize_and_execute(call).await.unwrap().content
    );
    let actual = std::fs::read_to_string(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    assert_eq!(actual, "Frozen manuscript");
    let delivery = tools
        .finalization_call()
        .await
        .unwrap()
        .expect("changed files require delivery before the final answer");
    assert_eq!(delivery.name, "research_deliver_files");
    // This synthetic host has no delivery dispatcher. Preserve the safe cause
    // before the tool adapter conservatively requires reconciliation.
    let delivery_scope = crate::agent::writing_workspace_commands::Scope {
        conversation_id: receipt.conversation_id,
        request_id: receipt.request_id,
        manuscript_id: receipt.manuscript_id,
        context_snapshot_id: context,
    };
    let delivery_binding = crate::agent::writing_workspace_commands::Confirmation {
        scope: delivery_scope.clone(),
        turn_id,
        lease_id: Uuid::new_v4(),
        manifest_hash: "a".repeat(64),
    };
    assert_eq!(
        f.store
            .writing_delivery_tools()
            .deliver(delivery_scope, turn_id, delivery_binding,)
            .await
            .err()
            .as_deref(),
        Some("file delivery host unavailable")
    );
    assert!(
        matches!(
            tools.authorize_and_execute(delivery).await,
            Err(research_agent_core::runner::RunError::ToolOutcomeUnknown)
        ),
        "unconfirmed delivery must preserve the sealed workspace for reconciliation"
    );
    // The still-running turn's sealed copy remains publishable through the
    // production review path; revoking execution must not revoke delivery.
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = service.review_workspace(receipt, context).await.unwrap();
    assert_eq!(review.changes.len(), 1);
    service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    assert!(
        tools.finalization_call().await.unwrap().is_none(),
        "a failed finalizer must not loop automatically"
    );
    assert_eq!(
        std::fs::read_to_string(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        "Frozen manuscript"
    );
    assert!(tools.validate(&ToolCall{
namespace: None,
raw_arguments_json: None,call_id:"legacy-downgrade".into(),name:"research_apply_patch".into(),arguments:serde_json::json!({"operations":[{"path":"main.tex","before":"Changed manuscript","after":"bad"}]})}).is_err());
}
