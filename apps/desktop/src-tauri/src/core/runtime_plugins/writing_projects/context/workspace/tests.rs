use super::*;
use crate::{
    agent::{
        repository::execution_workspaces,
        writing_submission::{
            WritingSubmissionService, WritingTestFixture as Fixture,
            writing_test_fixture as fixture, writing_test_request as request,
        },
        writing_workspace_commands::{self, PrepareRequest, Scope},
    },
    runtime_plugins::{RuntimePluginCatalog, native::NativePreviewService},
};
use research_agent_core::submission::SubmissionRecord;
use research_workbench_native_sdk::{
    data::DataValue, writing_context::ContextInput, writing_project::Input,
};
use std::{future::Future, sync::Arc, task::Poll, time::Duration};

async fn input() -> (Fixture, SubmissionRecord, Binding) {
    input_with_scope(false).await
}
async fn input_with_scope(dynamic: bool) -> (Fixture, SubmissionRecord, Binding) {
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
    let mut request = request(&f, Uuid::new_v4());
    if dynamic {
        request.resources = vec![
            crate::agent::context_session::sources::Resource::Manuscript {
                resource_id: f.manuscript.to_string(),
                new_member_access: Some(crate::agent::context_session::sources::Access::ReadWrite),
                group_access: Some(crate::agent::context_session::sources::Access::ReadWrite),
                parts: vec![],
            },
        ];
    }
    let prepared = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, request)
        .await
        .unwrap();
    (f, prepared.record, binding)
}
fn scope(f: &Fixture, record: &SubmissionRecord) -> Scope {
    Scope {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
        context_snapshot_id: record.context_snapshot_id,
    }
}
fn absent(f: &Fixture, record: &SubmissionRecord) {
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    assert!(
        execution_workspaces::read(&db, record.conversation_id, record.request_id)
            .unwrap()
            .is_none()
    );
    assert!(!f.store.agent_workspace_directory().exists());
}
async fn rejected(
    store: &AppStore,
    f: &Fixture,
    record: &SubmissionRecord,
    code: &str,
) -> serde_json::Value {
    let result = writing_workspace_commands::prepare(
        store,
        PrepareRequest {
            scope: scope(f, record),
            buffers: vec![],
        },
    )
    .await;
    let error = match result {
        Ok(_) => panic!("source rejection must not prepare a copy"),
        Err(error) => error,
    };
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], code, "{value}");
    assert_eq!(value["conversationId"], record.conversation_id.to_string());
    assert_eq!(value["manuscriptId"], f.manuscript.to_string());
    assert_eq!(value["operationId"], record.request_id.to_string());
    absent(f, record);
    value
}

#[tokio::test]
#[ignore = "requires actual independent writing package; explicit C04/C05 initial-copy acceptance"]
async fn actual_package_initial_copy_rejections_keep_authorization_and_safe_command_diagnostics() {
    let (f, record, binding) = input().await;
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let mut foreign = scope(&f, &record);
    foreign.conversation_id = Uuid::new_v4();
    let refused = writing_workspace_commands::prepare(
        &missing,
        PrepareRequest {
            scope: foreign,
            buffers: vec![],
        },
    )
    .await;
    assert_eq!(
        serde_json::to_value(refused.err().expect("foreign binding")).unwrap(),
        "denied"
    );
    rejected(&missing, &f, &record, "plugin_host_package_unavailable").await;
    let stopped = Arc::new(NativePreviewService::new(binding.catalog.clone()));
    stopped.shutdown().await.unwrap();
    let stopped_store = f.store.clone().with_native_writing_context(Binding {
        service: stopped,
        ..binding.clone()
    });
    rejected(&stopped_store, &f, &record, "plugin_host_stopping").await;
    f.store
        .connection()
        .unwrap()
        .execute(
            "update manuscripts set status='archived' where id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    let inactive = writing_workspace_commands::prepare(
        &missing,
        PrepareRequest {
            scope: scope(&f, &record),
            buffers: vec![],
        },
    )
    .await;
    assert_eq!(
        serde_json::to_value(inactive.err().expect("inactive manuscript")).unwrap(),
        "denied"
    );
    absent(&f, &record);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update manuscripts set status='active' where id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    // Deliberate isolated corruption exercises the original SQL failure path.
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop table manuscript_files")
        .unwrap();
    let sql = rejected(&f.store, &f, &record, "storage_error").await;
    assert_eq!(sql["dataError"]["resourceId"], "writing.projects.files");
    assert!(sql["dataError"]["sqliteExtendedCode"].is_number());
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

fn attempt(
    f: &Fixture,
    record: &SubmissionRecord,
    source: &mut WorkspaceSource,
) -> Result<Attempt<ContextNeed>, Error> {
    source.begin();
    preparation::prepare(
        &f.store,
        WritingSubmissionReceipt {
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            manuscript_id: f.manuscript,
        },
        record.context_snapshot_id,
        f.manuscript,
        |db, root, scope| source.capture(db, root, scope),
    )
}
fn need(result: Result<Attempt<ContextNeed>, Error>) -> ContextNeed {
    match result.unwrap() {
        Attempt::Need(input) => input,
        Attempt::Ready(_) => panic!("expected source pause before file creation"),
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; explicit C04/C05 read-prefix acceptance"]
async fn actual_package_initial_copy_rechecks_the_complete_prefix_before_creating_and_retains_the_source_gate()
 {
    let (f, record, binding) = input().await;
    let mut source = WorkspaceSource::new(
        binding.clone(),
        record.request_id,
        WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![],
        },
    );
    let project = need(attempt(&f, &record, &mut source));
    assert!(
        matches!(project.as_ref(), Input::Context { input } if matches!(input.as_ref(), ContextInput::Project { .. }))
    );
    absent(&f, &record);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update manuscripts set title='Changed after first read' where id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    source.resolve(project).await.unwrap();
    let changed = need(attempt(&f, &record, &mut source));
    assert!(
        matches!(changed.as_ref(), Input::Context { input } if matches!(input.as_ref(), ContextInput::Project { rows, .. }
        if rows.rows[0][2] == DataValue::Text("Changed after first read".into())))
    );
    absent(&f, &record);
    source.resolve(changed).await.unwrap();
    let files = need(attempt(&f, &record, &mut source));
    assert!(matches!(files.as_ref(), Input::ListFiles { .. }));
    absent(&f, &record);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update manuscript_files set modified_at='refreshed' where manuscript_id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    source.resolve(files).await.unwrap();
    let changed = need(attempt(&f, &record, &mut source));
    assert!(
        matches!(changed.as_ref(), Input::ListFiles { files: Some(rows), .. }
        if rows.rows[0][5] == DataValue::Text("refreshed".into()))
    );
    absent(&f, &record);
    source.resolve(changed).await.unwrap();
    let prepared = match attempt(&f, &record, &mut source).unwrap() {
        Attempt::Ready(prepared) => prepared,
        Attempt::Need(_) => panic!("unchanged prefix must be ready"),
    };
    assert_eq!(
        std::fs::read(prepared.lease.working_directory().unwrap().join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    let mut closing = Box::pin(binding.service.shutdown());
    let polled = std::future::poll_fn(|cx| Poll::Ready(closing.as_mut().poll(cx))).await;
    assert!(polled.is_pending());
    drop(source);
    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .unwrap()
        .unwrap();
    drop(prepared);
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; successor command diagnostics"]
async fn actual_package_successor_command_keeps_authorization_and_native_failure_identity() {
    use writing_workspace_commands::PrepareNextRequest;
    let (f, record, binding) = input().await;
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    for foreign in [true, false] {
        let mut current = scope(&f, &record);
        if foreign {
            current.conversation_id = Uuid::new_v4();
        }
        let result = writing_workspace_commands::prepare_next(
            &missing,
            PrepareNextRequest {
                scope: current,
                generation: 1,
                predecessor_delivery_id: Uuid::new_v4(),
                lease_id: Uuid::new_v4(),
                buffers: vec![],
            },
        )
        .await;
        let value = serde_json::to_value(result.err().expect("missing source")).unwrap();
        if foreign {
            assert_eq!(value, "denied");
        } else {
            assert_eq!(value["code"], "plugin_host_package_unavailable");
            assert_eq!(value["conversationId"], record.conversation_id.to_string());
            assert_eq!(value["manuscriptId"], f.manuscript.to_string());
            assert_eq!(value["operationId"], record.request_id.to_string());
        }
        absent(&f, &record);
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_agent_workspace_generations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[path = "tests/application.rs"]
mod application;

#[path = "tests/dynamic_scope.rs"]
mod dynamic_scope;
