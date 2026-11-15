//! Synthetic toolchain I/O, actual shared owner, sandbox and process registry.
use super::*;
use crate::agent::{
    repository::SqliteConversationStore,
    text_tools::file_workspace::FileWorkspaceSource,
    writing_submission::tests::{Fixture, fixture, request},
};
use research_agent_core::tool_registry::OwnedToolRegistration;
use std::os::unix::fs::PermissionsExt;

#[path = "owned_tests/adapter.rs"]
mod adapter;

#[path = "owned_tests/denial.rs"]
mod denial;
#[path = "owned_tests/retention.rs"]
mod retention;

struct Harness {
    fixture: Fixture,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    compiler: CompileWorkspaceTools,
    source: Arc<dyn FileWorkspaceSource>,
    tools: Arc<dyn ToolExecutor>,
    work: PathBuf,
}
impl Harness {
    async fn new() -> Self {
        let fixture = fixture(true);
        let service = WritingSubmissionService::from_app_store(&fixture.store);
        let record = service
            .prepare(&fixture.models, request(&fixture, Uuid::new_v4()))
            .await
            .unwrap()
            .record;
        let prepared = service
            .prepare_workspace(
                WritingSubmissionReceipt {
                    conversation_id: record.conversation_id,
                    request_id: record.request_id,
                    manuscript_id: fixture.manuscript,
                },
                record.context_snapshot_id,
                crate::agent::writing_workspace::WritingWorkspaceRequest {
                    manuscript_id: fixture.manuscript,
                    buffers: vec![],
                },
            )
            .await
            .unwrap();
        let prepared = service.approve_workspace(prepared).await.unwrap();
        let work = prepared.lease.working_directory().unwrap();
        drop(prepared);
        let sessions = Arc::new(AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(
                fixture.store.shared_agent_connection(),
                "compile-owner".into(),
            )
            .unwrap(),
        )));
        sessions
            .processes()
            .begin_turn(record.conversation_id, record.turn_id)
            .unwrap();
        let mut writing =
            super::super::for_submission(fixture.store.clone(), record.clone(), sessions.clone())
                .await
                .unwrap();
        let source = writing.file_workspace().unwrap();
        writing.compile_workspace.as_mut().unwrap().toolchain = Some(toolchain(&fixture));
        let compiler = writing.compile_workspace.as_ref().unwrap().clone();
        let registration =
            crate::agent::context_session::resource_tools::registration_with_workspace(
                fixture.store.clone(),
                record.clone(),
                OwnedToolRegistration::new(writing.definitions(), Arc::new(writing)),
                Some(source.clone()),
            );
        Self {
            fixture,
            record,
            sessions,
            compiler,
            source,
            tools: registration.into_parts().1,
            work,
        }
    }
    fn path(&self) -> String {
        format!("source://writings/{}/main.tex", self.fixture.manuscript)
    }
    fn call(&self, id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: id.into(),
            name: name.into(),
            arguments,
        }
    }
    fn compile_call(&self, id: &str) -> ToolCall {
        self.call(
            id,
            COMPILE_WORKSPACE,
            json!({"rootPath":"main.tex","engine":"pdflatex"}),
        )
    }
    async fn execute(&self, call: ToolCall) -> serde_json::Value {
        let result = self
            .tools
            .authorize_and_execute(call.clone())
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        self.tools.result_committed(call).await.unwrap();
        serde_json::from_str(&result.content).unwrap()
    }
    async fn read(&self) {
        self.execute(self.call("read", "research_read_file", json!({"path":self.path()})))
            .await;
    }
    fn outputs(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.work)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".research-assistant-compile-")
            })
            .collect()
    }
    fn cached_source(&self, result: &serde_json::Value) -> String {
        std::fs::read_to_string(
            self.fixture
                .store
                .writing_cache_directory()
                .join("agent-compile-results")
                .join(result["resultId"].as_str().unwrap())
                .join("source/main.tex"),
        )
        .unwrap()
    }
    fn original(&self) -> String {
        std::fs::read_to_string(
            self.fixture
                .store
                .manuscript_directory()
                .join(self.fixture.manuscript.to_string())
                .join("source/main.tex"),
        )
        .unwrap()
    }
}

fn toolchain(fixture: &Fixture) -> LatexToolchainInfo {
    let directory = fixture.root.path().join("synthetic-toolchain");
    std::fs::create_dir(&directory).unwrap();
    let script = directory.join("latexmk");
    std::fs::write(&script, b"#!/bin/sh\nset -eu\nout=\nfor arg in \"$@\"; do case \"$arg\" in -outdir=*) out=${arg#-outdir=};; esac; done\ntest -n \"$out\"\nprintf '%%PDF-1.4\\nsynthetic artifact\\n%%%%EOF\\n' > \"$out/main.pdf\"\nprintf synthetic-synctex > \"$out/main.synctex.gz\"\nprintf compile-complete\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    serde_json::from_value(json!({"available":true,"tools":[{"name":"latexmk","available":true,
        "path":script.to_string_lossy(),"version":"synthetic I/O"}],"supportedEngines":["pdflatex"],"checkedAt":"synthetic"})).unwrap()
}

#[tokio::test]
async fn file_first_compile_and_patch_use_original_owner_and_leave_no_generated_changes() {
    let h = Harness::new().await;
    h.read().await;
    let owned = h.source.workspace(&h.record).await.unwrap();
    let result = h.execute(h.compile_call("compile-after-file")).await;
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["retained"], true);
    assert_eq!(
        result["identity"]["leaseId"],
        owned.binding.lease_id.to_string()
    );
    assert_eq!(
        result["identity"]["requestId"],
        h.record.request_id.to_string()
    );
    assert_eq!(h.cached_source(&result), "Frozen manuscript");
    assert!(h.outputs().is_empty());
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n-Frozen manuscript\n+after compile\n*** End Patch",
        h.path()
    );
    let changed = h
        .execute(h.call(
            "patch-after-compile",
            "research_apply_patch",
            json!({"patch":patch}),
        ))
        .await;
    assert_eq!(changed["targets"][0]["status"], "applied");
    let changes = owned.host.changes().await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "main.tex");
    assert!(h.tools.finalization_call().await.unwrap().is_some());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn compile_first_does_not_fake_delivery_and_later_exec_compile_keeps_exact_source() {
    let h = Harness::new().await;
    let first = h.execute(h.compile_call("compile-first")).await;
    assert_eq!(first["status"], "succeeded");
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    let shell = h
        .execute(h.call(
            "exec-after-compile",
            crate::agent::shell_tools::EXEC,
            json!({"command":"printf exec-copy > main.tex", "yield_ms":5000}),
        ))
        .await;
    assert_eq!(shell["state"], "finished");
    assert_eq!(shell["exitCode"], 0);
    let second = h.execute(h.compile_call("compile-after-exec")).await;
    assert_eq!(second["status"], "succeeded");
    assert_eq!(second["identity"], first["identity"]);
    assert_ne!(second["resultId"], first["resultId"]);
    assert_eq!(h.cached_source(&first), "Frozen manuscript");
    assert_eq!(h.cached_source(&second), "exec-copy");
    h.read().await;
    assert!(h.outputs().is_empty());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
