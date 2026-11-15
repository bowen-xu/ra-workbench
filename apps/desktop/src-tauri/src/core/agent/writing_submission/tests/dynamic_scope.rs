use super::*;

#[tokio::test]
async fn dynamic_project_preparation_inherits_new_files_and_keeps_buffers_and_overrides() {
    use crate::agent::context_session::{
        group_scope, permissions,
        sources::{Access, Part, Resource},
    };
    let f = fixture(true);
    let mut input = request(&f, Uuid::new_v4());
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: None,
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: "main.tex".into(),
            },
            label: "Main".into(),
            selected: true,
            access: Access::ReadOnly,
        }],
    }];
    // Added after the saved authorization; no explicit file grant is appended.
    let text = "New member";
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    f.store.connection().unwrap().execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,'new.tex','tex',?2,?3,'now')",params![f.manuscript.to_string(),text.len(),hash]).unwrap();
    std::fs::write(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/new.tex"),
        text,
    )
    .unwrap();
    let expected = input.context.files[0]
        .buffer
        .as_ref()
        .map(|buffer| buffer.text.clone())
        .unwrap_or_else(|| "Frozen manuscript".into());
    let prepared = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, input)
        .await
        .unwrap();
    let current = &prepared.record.input.context;
    assert_eq!(
        current
            .iter()
            .find(|source| source.resource_id == "main.tex")
            .unwrap()
            .text,
        expected
    );
    let new = current
        .iter()
        .find(|source| source.resource_id == "new.tex")
        .unwrap();
    assert_eq!(new.text, text);
    let anchor: serde_json::Value = serde_json::from_str(new.anchor.as_deref().unwrap()).unwrap();
    assert_eq!(anchor["contextAccess"], "readWrite");
    // The original composer reference stays readable without duplicating inventory/body entries.
    let registered = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        prepared.record.clone(),
        research_agent_core::tool_registry::OwnedToolRegistration::new(
            vec![],
            std::sync::Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
        ),
    );
    let (_, executor) = registered.into_parts();
    let list = executor
        .authorize_and_execute(research_agent_core::model::ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "scope-list".into(),
            name: "research_list_sources".into(),
            arguments: serde_json::json!({}),
        })
        .await
        .unwrap();
    assert!(!list.is_error);
    let listed: serde_json::Value = serde_json::from_str(&list.content).unwrap();
    assert_eq!(listed["total"], 2);
    assert!(!list.content.contains("Frozen manuscript"));
    let main = current
        .iter()
        .find(|source| source.resource_id == "main.tex")
        .unwrap();
    let read = executor
        .authorize_and_execute(research_agent_core::model::ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "scope-read".into(),
            name: "research_read_source".into(),
            arguments: serde_json::json!({"sourceId":main.source_id,"maxChars":8000}),
        })
        .await
        .unwrap();
    assert!(!read.is_error);
    assert!(read.content.contains("Frozen manuscript"));
    let scope = permissions::writing_scope(&prepared.record).unwrap();
    assert!(group_scope::path_access(
        &scope,
        "future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(!group_scope::path_access(
        &scope,
        "main.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(current.iter().any(|source| {
        crate::agent::context_session::resource_grants::resource(source)
            .is_some_and(|resource| group_scope::is_dynamic(&resource))
    }));
}

#[tokio::test]
async fn a_directory_scope_with_all_current_files_excluded_keeps_its_permission_ceiling() {
    use crate::agent::context_session::{
        group_scope, permissions,
        sources::{Access, Part, Resource},
    };
    let f = fixture(true);
    let mut input = request(&f, Uuid::new_v4());
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: None,
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: "main.tex".into(),
            },
            label: "Main".into(),
            selected: false,
            access: Access::ReadWrite,
        }],
    }];
    let prepared = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, input)
        .await
        .unwrap();
    assert!(
        prepared
            .record
            .input
            .context
            .iter()
            .all(|source| source.resource_id != "main.tex")
    );
    assert_eq!(
        permissions::writing_paths(&prepared.record),
        Some(std::collections::HashSet::new())
    );
    let scope = permissions::writing_scope(&prepared.record).unwrap();
    assert!(!group_scope::path_access(
        &scope,
        "main.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(group_scope::path_access(
        &scope,
        "future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
}
