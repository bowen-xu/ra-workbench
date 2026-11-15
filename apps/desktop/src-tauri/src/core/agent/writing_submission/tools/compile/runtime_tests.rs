//! The actual original Agent executor with static compile sources absent.
use super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
};
use std::{sync::Arc, time::Duration};

fn selected() -> Binding {
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
    Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: listing.packages[0].package_key.clone(),
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "runtime-feedback".into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package and installed TeX; explicit C05 Agent acceptance"]
async fn actual_package_agent_feedback_keeps_authorization_history_currentness_and_bounded_output()
{
    let mut f = crate::agent::writing_submission::tests::fixture(true);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    let binding = selected();
    let unbound = CompileTools {
        store: f.store.clone().without_static_writing_compile_sources(),
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
    };
    assert!(matches!(
        unbound
            .authorize_and_execute(call(READ_DIAGNOSTICS, json!({})))
            .await,
        Err(RunError::ToolRejected)
    ));
    let missing = CompileTools {
        store: unbound.store.clone().with_native_writing_context(Binding {
            package_key: "0".repeat(64),
            ..binding.clone()
        }),
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
    };
    let refused = missing
        .authorize_and_execute(call(READ_DIAGNOSTICS, json!({})))
        .await
        .unwrap();
    assert!(refused.is_error && refused.content.contains("plugin_host_package_unavailable"));
    f.store = f
        .store
        .without_static_writing_compile_sources()
        .with_native_writing_context(binding.clone());
    let tools = CompileTools {
        store: f.store.clone(),
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
    };
    let empty = tools
        .authorize_and_execute(call(READ_DIAGNOSTICS, json!({})))
        .await
        .unwrap();
    assert!(!empty.is_error);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&empty.content).unwrap(),
        json!({"status":"no_compile_run"})
    );
    let text =
        "\\documentclass{article}\n\\begin{document}\nRuntime Agent feedback.\n\\end{document}\n";
    let hash = f
        .store
        .manuscript_files(&f.manuscript.to_string())
        .save("main.tex", text, None)
        .unwrap();
    f.store.connection().unwrap().execute("update manuscript_files set sha256=?1,byte_size=?2 where manuscript_id=?3 and relative_path='main.tex'", rusqlite::params![hash,text.len() as i64,f.manuscript.to_string()]).unwrap();
    let queued = crate::runtime_plugins::writing_projects::compile::start::start(
        f.store.clone(),
        binding.clone(),
        crate::compile_activity::CompileManager::default(),
        f.manuscript.to_string(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(150), async {
        loop {
            let status: String = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "select status from manuscript_compile_runs where id=?1",
                    [&queued.id],
                    |row| row.get(0),
                )
                .unwrap();
            if !matches!(status.as_str(), "queued" | "running") {
                assert_eq!(status, "succeeded");
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .unwrap();
    let diagnostics = tools
        .authorize_and_execute(call(READ_DIAGNOSTICS, json!({"includeRawOutput":true})))
        .await
        .unwrap();
    assert!(!diagnostics.is_error, "{}", diagnostics.content);
    let value: serde_json::Value = serde_json::from_str(&diagnostics.content).unwrap();
    assert_eq!(value["runId"], queued.id);
    assert_eq!(value["status"], "succeeded");
    assert!(value["rawOutput"].as_str().unwrap().chars().count() <= MAX_RAW_OUTPUT + 1);
    let forward = call(FORWARD_SYNCTEX, json!({"relativePath":"main.tex","line":3}));
    let located = tools.authorize_and_execute(forward.clone()).await.unwrap();
    assert!(!located.is_error, "{}", located.content);
    assert!(
        !serde_json::from_str::<Vec<crate::agent::compile_synctex::SynctexPdfLocation>>(
            &located.content
        )
        .unwrap()
        .is_empty()
    );
    f.store
        .manuscript_files(&f.manuscript.to_string())
        .save("main.tex", &(text.to_owned() + "% later edit\n"), None)
        .unwrap();
    let stale = tools.authorize_and_execute(forward).await.unwrap();
    assert!(stale.is_error);
    assert!(stale.content.starts_with("SyncTeX unavailable: "));
    assert!(stale.content.contains("synctex_unavailable"));
    let diagnostic: serde_json::Value =
        serde_json::from_str(stale.content.split_once(": ").unwrap().1).unwrap();
    assert_eq!(diagnostic["packageKey"], binding.package_key);
    assert_eq!(diagnostic["pluginId"], "writing_projects");
    assert!(diagnostic["instanceId"].as_str().is_some());
    assert!(diagnostic["requestId"].as_str().is_some());
    binding.service.shutdown().await.unwrap();
    let stopped = tools
        .authorize_and_execute(call(READ_DIAGNOSTICS, json!({})))
        .await
        .unwrap();
    assert!(stopped.is_error && stopped.content.contains("plugin_host_stopping"));
    assert!(stopped.content.contains(&binding.package_key));
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert!(matches!(
        tools
            .authorize_and_execute(call(READ_DIAGNOSTICS, json!({})))
            .await,
        Err(RunError::ToolRejected)
    ));
    f.store.shared_agent_connection().shutdown().unwrap();
}
