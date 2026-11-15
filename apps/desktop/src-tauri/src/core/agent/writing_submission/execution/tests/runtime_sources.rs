//! Native sources take the same existing start boundary as static sources.
use super::*;
use crate::{
    agent::context_session::sources::{Access, Resource},
    runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    },
};

#[tokio::test]
#[ignore = "requires actual independent writing package; original startup source selection"]
async fn actual_package_start_validation_accepts_native_sources_and_keeps_original_rejections() {
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
    let missing = f
        .store
        .clone()
        .without_static_writing_context_sources()
        .without_static_writing_workspace_source()
        .without_static_writing_compile_sources();
    f.store = missing.clone().with_native_writing_context(binding.clone());
    assert!(f.store.writing_resource_source().is_none());
    assert!(f.store.writing_compile_feedback_source().is_none());
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut input = request(&f, Uuid::new_v4());
    input.resources = vec![Resource::Manuscript {
        resource_id: f.manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![],
    }];
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let confirmed = confirmation(&prepared.record, f.manuscript);
    let selected = profile(prepared.record.input.model.clone());
    service
        .validate_start(&prepared, &confirmed, &selected)
        .await
        .unwrap();
    let unbound = WritingSubmissionService::from_app_store(&missing);
    assert!(matches!(
        unbound
            .validate_start(&prepared, &confirmed, &selected)
            .await,
        Err(WritingStartError::Preparation(
            WritingSubmissionError::Unavailable
        ))
    ));
    let mut stale = confirmation(&prepared.record, f.manuscript);
    stale.context_snapshot_id = Uuid::new_v4();
    assert!(matches!(
        service.validate_start(&prepared, &stale, &selected).await,
        Err(WritingStartError::Mismatch)
    ));
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(matches!(
        service
            .validate_start(&prepared, &confirmed, &selected)
            .await,
        Err(WritingStartError::Preparation(
            WritingSubmissionError::Unbound
        ))
    ));
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
