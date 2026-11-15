//! Actual independently built package, Core storage and existing tool registration.
use super::*;
use crate::agent::composed_tools::RejectingToolExecutor;
use research_agent_core::{model::ToolCall, tool_registry::OwnedToolRegistration};
use serde_json::{Value, json};

fn call(name: &str, id: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; dynamic scope prepare/read/edit chain"]
async fn actual_scope_prepares_lists_reads_searches_and_edits_only_the_approved_copy() {
    edit_approved_copy(true).await;
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original explicit-file copy tools"]
async fn actual_explicit_files_keep_original_approved_copy_tool_behavior() {
    edit_approved_copy(false).await;
}

async fn edit_approved_copy(dynamic: bool) {
    let (f, record, binding) = input_with_scope(dynamic).await;
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    assert!(f.store.writing_resource_source().is_none());
    assert_eq!(
        crate::agent::context_session::permissions::writing_scope(&record).is_some(),
        dynamic
    );
    let service = WritingSubmissionService::from_app_store(&f.store);
    let receipt = WritingSubmissionReceipt {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let working = copy.lease.working_directory().unwrap().to_path_buf();
    drop(service.approve_workspace(copy).await.unwrap());
    let (_, tools) = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        record.clone(),
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .into_parts();
    let read = tools
        .authorize_and_execute(call(
            "research_read_file",
            "read-file",
            json!({"path":format!("source://writings/{}/main.tex",f.manuscript)}),
        ))
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.content);
    let patch = format!(
        "*** Begin Patch\n*** Update File: source://writings/{}/main.tex\n@@\n-Frozen manuscript\n+Changed manuscript\n*** Add File: source://writings/{}/sections/new.tex\n+New member\n*** End Patch",
        f.manuscript, f.manuscript
    );
    let output = tools
        .authorize_and_execute(call(
            "research_apply_patch",
            "patch",
            json!({"patch":patch}),
        ))
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    assert_eq!(
        std::fs::read_to_string(working.join("main.tex")).unwrap(),
        "Changed manuscript\n"
    );
    assert_eq!(
        std::fs::read_to_string(working.join("sections/new.tex")).unwrap(),
        "New member\n"
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
    let denied = tools
        .authorize_and_execute(call(
            "research_read_file",
            "denied",
            json!({"path":format!("source://writings/{}/image.pdf",f.manuscript)}),
        ))
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error);
    let review = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(review.changes.len(), 2);
    service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    drop(tools);
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; current manuscript source tools"]
async fn actual_scope_source_tools_list_read_and_search_current_files() {
    let (f, record, binding) = input_with_scope(true).await;
    let main = record
        .input
        .context
        .iter()
        .find(|source| source.resource_id == "main.tex")
        .unwrap();
    let registered = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        record.clone(),
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    );
    let (_, tools) = registered.into_parts();
    for request in [
        call("research_list_sources", "list", json!({})),
        call(
            "research_read_source",
            "read-source",
            json!({"sourceId":main.source_id,"maxChars":8000}),
        ),
        call(
            "research_search_sources",
            "search",
            json!({"query":"Frozen"}),
        ),
    ] {
        let name = request.name.clone();
        let output = tools.authorize_and_execute(request).await.unwrap();
        assert!(!output.is_error, "{name}: {}", output.content);
        if name == "research_list_sources" {
            let listed: Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(listed["total"], 1);
            assert!(!output.content.contains("Frozen manuscript"));
        } else {
            assert!(
                output.content.contains("Frozen"),
                "{name}: {}",
                output.content
            );
        }
    }
    drop(tools);
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
async fn static_dynamic_scope_registers_the_original_copy_and_retains_approval() {
    use crate::agent::context_session::sources::{Access, Resource};
    let f = fixture(true);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    let mut input = request(&f, Uuid::new_v4());
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![],
    }];
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service.prepare(&f.models, input).await.unwrap().record;
    let result = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: record.conversation_id,
                request_id: record.request_id,
                manuscript_id: f.manuscript,
            },
            record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await;
    let prepared = result.unwrap();
    assert_eq!(
        execution_workspaces::authorize(
            &f.store.connection().unwrap(),
            &prepared.binding,
            prepared.lease.manifest()
        ),
        Err(execution_workspaces::Error::Denied)
    );
    drop(prepared);
}
