//! Compile schema and actual installed-toolchain smoke coverage.
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceRequest;
use crate::agent::{
    repository::SqliteConversationStore, service::AgentSessionService,
    writing_submission::WritingSubmissionService,
};
use research_agent_core::runner::ToolExecutor;

#[test]
fn accepts_only_relative_tex_roots_and_known_engines() {
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "compile-1".into(),
        name: COMPILE_WORKSPACE.into(),
        arguments: json!({"rootPath": "paper/main.tex", "engine": "pdflatex"}),
    };
    assert!(CompileWorkspaceTools::parse(&call).is_ok());
    for arguments in [
        json!({"rootPath": "../main.tex", "engine": "pdflatex"}),
        json!({"rootPath": "/tmp/main.tex", "engine": "pdflatex"}),
        json!({"rootPath": "main.tex", "engine": "shell"}),
        json!({"rootPath": "main.tex", "engine": "pdflatex", "command": "rm -rf ."}),
    ] {
        let mut invalid = call.clone();
        invalid.arguments = arguments;
        assert!(CompileWorkspaceTools::parse(&invalid).is_err());
    }
}

#[test]
fn does_not_deny_an_app_config_ancestor_of_the_work_copy() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let work = root.join("agent-workspaces/lease/work");
    let protected = protected_paths_for_roots(&root, &root, &work).unwrap();
    assert!(!protected.iter().any(|path| path == &root));
    assert!(
        protected
            .iter()
            .any(|path| path == &root.join("manuscripts"))
    );
}

#[test]
fn rejects_a_configuration_directory_inside_the_work_copy() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let work = root.join("agent-workspaces/lease/work");
    assert!(protected_paths_for_roots(&root, &work.join("config"), &work).is_err());
}

#[test]
fn removes_only_a_real_direct_ephemeral_directory() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("main.log"), "result").unwrap();
    if output != output.canonicalize().unwrap() {
        assert!(remove_ephemeral_directory(&output).is_err());
    }
    let output = output.canonicalize().unwrap();
    remove_ephemeral_directory(&output).unwrap();
    assert!(!output.exists());
}

#[test]
fn output_tail_is_unicode_safe() {
    let text = format!("{}中文", "x".repeat(OUTPUT_LIMIT));
    let value = tail(&text, OUTPUT_LIMIT);
    assert_eq!(value.chars().count(), OUTPUT_LIMIT + 1);
    assert!(value.ends_with("中文"));
}

#[tokio::test]
async fn compiles_the_approved_copy_without_touching_the_live_source() {
    compile_approved_copy(None).await;
}

#[tokio::test]
#[ignore = "requires actual independent writing package and installed TeX; runtime initial-copy preparation"]
async fn actual_package_compiles_the_approved_copy_with_runtime_rules_and_original_sandbox() {
    use crate::runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    };
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
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    compile_approved_copy(Some(Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: listing.packages[0].package_key.clone(),
    }))
    .await;
}

async fn compile_approved_copy(
    native: Option<crate::runtime_plugins::writing_projects::context::Binding>,
) {
    let mut fixture = crate::agent::writing_submission::tests::fixture(true);
    if native.is_none()
        && detect_toolchain(
            &fixture.store,
            fixture.store.writing_workspace_compile_source().unwrap(),
        )
        .map(|toolchain| !toolchain.available)
        .unwrap_or(true)
    {
        return;
    }
    if let Some(binding) = &native {
        crate::storage::migrate(&mut fixture.store.connection().unwrap()).unwrap();
        fixture.store = fixture
            .store
            .without_static_writing_context_sources()
            .without_static_writing_workspace_source()
            .without_static_writing_compile_sources()
            .with_native_writing_context(binding.clone());
    }
    let service = WritingSubmissionService::from_app_store(&fixture.store);
    let submitted = service
        .prepare(
            &fixture.models,
            crate::agent::writing_submission::tests::request(&fixture, Uuid::new_v4()),
        )
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: fixture.conversation,
        request_id: submitted.record.request_id,
        manuscript_id: fixture.manuscript,
    };
    let prepared = service
        .prepare_workspace(
            receipt,
            submitted.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: fixture.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = service.approve_workspace(prepared).await.unwrap();
    let scope = approved.lease.manifest().identity.clone();
    fs::write(
        approved.lease.working_directory().unwrap().join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\nAgent copy\n\\end{document}\n",
    )
    .unwrap();
    drop(approved);
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            fixture.store.shared_agent_connection(),
            "compile-test".into(),
        )
        .unwrap(),
    )));
    sessions
        .processes()
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let (_, host) = super::super::shell::for_submission(
        fixture.store.clone(),
        submitted.record.clone(),
        sessions.clone(),
    )
    .await
    .unwrap()
    .unwrap();
    let tools = CompileWorkspaceTools::for_submission(
        fixture.store.clone(),
        &submitted.record,
        sessions.clone(),
        host,
    )
    .await
    .unwrap()
    .expect("approved workspace tool");
    let output = tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "compile-live".into(),
            name: COMPILE_WORKSPACE.into(),
            arguments: json!({"rootPath":"main.tex","engine":"pdflatex"}),
        })
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    let result = serde_json::from_str::<serde_json::Value>(&output.content).unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["resultId"], result["compileVersion"]);
    assert_eq!(
        result["identity"]["conversationId"],
        fixture.conversation.to_string()
    );
    assert_eq!(
        result["identity"]["requestId"],
        submitted.record.request_id.to_string()
    );
    assert_eq!(
        result["identity"]["manuscriptId"],
        fixture.manuscript.to_string()
    );
    assert_eq!(
        result["identity"]["manifestHash"].as_str().map(str::len),
        Some(64)
    );
    let result_id = Uuid::parse_str(result["resultId"].as_str().unwrap()).unwrap();
    assert_eq!(
        fs::read(
            fixture
                .store
                .writing_cache_directory()
                .join("agent-compile-results")
                .join(result_id.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"\\documentclass{article}\n\\begin{document}\nAgent copy\n\\end{document}\n"
    );
    assert_eq!(
        fs::read(
            fixture
                .store
                .manuscript_directory()
                .join(fixture.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
    assert!(
        service
            .review_workspace(receipt, submitted.record.context_snapshot_id)
            .await
            .is_err()
    );
    let owned = tools.workspace.workspace(&submitted.record).await.unwrap();
    let store = fixture.store.clone();
    let binding = owned.binding.clone();
    let review = owned
        .host
        .inspect_owned(move |lease| {
            let db = store.connection().map_err(|_| RunError::EventSink)?;
            WritingSubmissionService::from_app_store(&store)
                .review_owned_workspace_binding(&db, &binding, lease)
                .map_err(|_| RunError::ToolRejected)
        })
        .await
        .unwrap();
    assert_eq!(review.changes.len(), 1);
    assert_eq!(review.changes[0].path, "main.tex");
    if native.is_some() {
        runtime_result_queries(&fixture.store, &owned.binding, result_id).await;
        let terminal = research_agent_core::protocol::EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: research_agent_core::protocol::TurnOutcome::Completed,
        };
        fixture
            .store
            .connection()
            .unwrap()
            .execute(
                "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) \
             select ?1,coalesce(max(ordinal)+1,0),1,?2 from agent_items where conversation_id=?1",
                rusqlite::params![
                    fixture.conversation.to_string(),
                    json!({"payload":terminal}).to_string()
                ],
            )
            .unwrap();
        runtime_result_queries(&fixture.store, &owned.binding, result_id).await;
        assert!(
            service
                .restore_workspace(receipt, submitted.record.context_snapshot_id)
                .await
                .is_err()
        );
    }
    sessions
        .processes()
        .finish_turn(scope.conversation_id, scope.turn_id)
        .await
        .unwrap();
    if let Some(binding) = native {
        assert_eq!(
            fixture
                .store
                .connection()
                .unwrap()
                .query_row("select count(*) from manuscript_compile_runs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        binding.service.shutdown().await.unwrap();
    }
}

async fn runtime_result_queries(
    store: &crate::storage::AppStore,
    binding: &crate::agent::repository::execution_workspaces::Binding,
    result_id: Uuid,
) {
    use crate::agent::compile_result_commands::{forward_synctex_in, inverse_synctex_in};
    use crate::agent::compile_synctex::AgentCompileSynctex;
    let locations = forward_synctex_in(
        store.clone(),
        None::<AgentCompileSynctex>,
        result_id.to_string(),
        binding.conversation_id.to_string(),
        binding.request_id.to_string(),
        binding.turn_id.to_string(),
        binding.context_snapshot_id.to_string(),
        binding.manuscript_id.to_string(),
        binding.lease_id.to_string(),
        binding.manifest_hash.clone(),
        "main.tex".into(),
        3,
        1,
    )
    .await
    .unwrap();
    assert!(!locations.is_empty());
    let location = &locations[0];
    let sources = inverse_synctex_in(
        store.clone(),
        None::<AgentCompileSynctex>,
        result_id.to_string(),
        binding.conversation_id.to_string(),
        binding.request_id.to_string(),
        binding.turn_id.to_string(),
        binding.context_snapshot_id.to_string(),
        binding.manuscript_id.to_string(),
        binding.lease_id.to_string(),
        binding.manifest_hash.clone(),
        location.page,
        location.x,
        location.y,
    )
    .await
    .unwrap();
    assert!(
        sources
            .iter()
            .any(|source| source.relative_path == "main.tex")
    );
}
