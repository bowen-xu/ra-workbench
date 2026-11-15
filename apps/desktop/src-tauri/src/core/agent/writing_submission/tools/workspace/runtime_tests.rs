//! Original tool entry, actual Native package and Core-owned approved copy.
use super::*;
use crate::{
    agent::{
        context_session::sources::{Access, Resource},
        writing_submission::{writing_test_fixture as fixture, writing_test_request as request},
        writing_workspace::WritingWorkspaceRequest,
    },
    runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    },
};

#[tokio::test]
#[ignore = "requires actual independent writing package; original workspace change reader"]
async fn actual_package_reads_authorized_changes_and_preserves_version_and_source_diagnostics() {
    let root = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
            .expect("actual independent writing package collection"),
    )
    .unwrap();
    assert!(
        root.starts_with(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .join(".tmp")
        )
    );
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)));
    let packages = catalog.discover().unwrap();
    assert_eq!(packages.packages.len(), 1);
    let binding = Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: packages.packages[0].package_key.clone(),
    };
    let mut f = fixture(true);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    f.store = f
        .store
        .without_static_writing_context_sources()
        .without_static_writing_workspace_source()
        .without_static_writing_compile_sources()
        .with_native_writing_context(binding.clone());
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut input = request(&f, Uuid::new_v4());
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![],
    }];
    let record = service.prepare(&f.models, input).await.unwrap().record;
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
    let root = copy.lease.working_directory().unwrap().to_path_buf();
    drop(service.approve_workspace(copy).await.unwrap());
    std::fs::write(root.join("main.tex"), b"Reviewed runtime text\n").unwrap();
    std::fs::write(root.join("new.tex"), b"Added runtime file\n").unwrap();
    let review = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(review.changes.len(), 2);
    let tools = WorkspaceTools::new(f.store.clone(), &record).unwrap();
    for change in &review.changes {
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: change.path.clone(),
            name: READ_CHANGE.into(),
            arguments: json!({"path":change.path,"before":change.before,"after":change.after}),
        };
        tools.validate(&call).unwrap();
        let output = tools.authorize_and_execute(call.clone()).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("runtime"));
        let mut stale = call.clone();
        stale.arguments["after"]["sha256"] = json!("f".repeat(64));
        assert!(matches!(
            tools.authorize_and_execute(stale).await,
            Err(RunError::ToolRejected)
        ));
        let missing = f.store.clone().with_native_writing_context(Binding {
            package_key: "0".repeat(64),
            ..binding.clone()
        });
        let unavailable = WorkspaceTools::new(missing, &record)
            .unwrap()
            .authorize_and_execute(call)
            .await
            .unwrap();
        assert!(unavailable.is_error);
        assert!(unavailable.content.contains(&"0".repeat(64)));
        assert!(unavailable.content.contains(&record.request_id.to_string()));
    }
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
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
