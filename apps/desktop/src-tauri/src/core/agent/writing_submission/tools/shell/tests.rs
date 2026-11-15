//! Actual lazy writing owner plus production resource/text registration.
use super::*;
use crate::agent::{
    repository::SqliteConversationStore,
    text_tools::file_workspace::FileWorkspaceSource,
    writing_submission::tests::{Fixture, fixture, request},
};
use research_agent_core::{
    model::ToolCall, runner::ToolExecutor, tool_registry::OwnedToolRegistration,
};
use serde_json::{Value, json};
use std::{os::unix::fs::MetadataExt, path::PathBuf};

#[path = "tests/denial.rs"]
mod denial;
#[path = "tests/journal_faults.rs"]
mod journal_faults;
#[path = "tests/retention.rs"]
mod retention;

struct Harness {
    fixture: Fixture,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    source: Arc<dyn FileWorkspaceSource>,
    tools: Arc<dyn ToolExecutor>,
    work: PathBuf,
}
impl Harness {
    async fn new() -> Self {
        Self::with_source(|source| source).await
    }
    async fn with_source(
        wrap: impl FnOnce(Arc<dyn FileWorkspaceSource>) -> Arc<dyn FileWorkspaceSource>,
    ) -> Self {
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
                "shared-file".into(),
            )
            .unwrap(),
        )));
        sessions
            .processes()
            .begin_turn(record.conversation_id, record.turn_id)
            .unwrap();
        let writing =
            super::super::for_submission(fixture.store.clone(), record.clone(), sessions.clone())
                .await
                .unwrap();
        let source = wrap(
            writing
                .file_workspace()
                .expect("approved workspace has the original lazy owner"),
        );
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
            source,
            tools: registration.into_parts().1,
            work,
        }
    }
    fn path(&self) -> String {
        format!("source://writings/{}/main.tex", self.fixture.manuscript)
    }
    fn call(&self, id: &str, name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: id.into(),
            name: name.into(),
            arguments,
        }
    }
    async fn execute(&self, call: ToolCall) -> (bool, Value) {
        let output = self
            .tools
            .authorize_and_execute(call.clone())
            .await
            .unwrap();
        self.tools.result_committed(call).await.unwrap();
        (
            output.is_error,
            serde_json::from_str(&output.content).unwrap(),
        )
    }
    async fn read(&self, id: &str) -> (bool, Value) {
        self.execute(self.call(id, "research_read_file", json!({"path":self.path()})))
            .await
    }
    fn patch(&self, id: &str, before: &str, after: &str) -> ToolCall {
        self.call(id, "research_apply_patch", json!({"patch":format!(
            "*** Begin Patch\n*** Update File: {}\n@@\n-{before}\n+{after}\n*** End Patch", self.path()
        )}))
    }
    fn read_count(&self) -> i64 {
        self.fixture
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_text_read_scopes", [], |row| {
                row.get(0)
            })
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

#[tokio::test]
async fn completed_shell_without_workspace_changes_needs_no_delivery() {
    let h = Harness::new().await;
    let result = h
        .execute(h.call(
            "read-only-shell",
            crate::agent::shell_tools::EXEC,
            json!({"command":"cat main.tex", "yield_ms":5000}),
        ))
        .await;
    assert!(!result.0, "{:?}", result.1);
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn delivery_cannot_strand_an_unacknowledged_finished_process() {
    let h = Harness::new().await;
    let call = h.call(
        "unacknowledged-shell",
        crate::agent::shell_tools::EXEC,
        json!({"command":"printf revised > main.tex", "yield_ms":5000}),
    );
    let result = h.tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(h.tools.release_for_finalization().await.is_err());
    h.tools.result_committed(call).await.unwrap();
    assert!(h.tools.release_for_finalization().await.unwrap());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn file_first_read_and_concurrent_initialization_share_one_owner_then_exec() {
    let h = Harness::new().await;
    // Concurrent first consumers must converge on the same OnceCell, not race
    // two independent restores or turn a read-only copy into a delivery.
    let (left, right, read) = tokio::join!(
        h.source.workspace(&h.record),
        h.source.workspace(&h.record),
        h.read("first-read"),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.binding, right.binding);
    assert!(!read.0, "{:?}", read.1);
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    let inode = std::fs::metadata(h.work.join("main.tex")).unwrap().ino();
    let result = h
        .execute(h.call(
            "exec-after-read",
            crate::agent::shell_tools::EXEC,
            json!({"command":"printf shell-copy > main.tex", "yield_ms":5000}),
        ))
        .await;
    assert!(!result.0, "{:?}", result.1);
    assert_eq!(result.1["state"], "finished");
    assert_eq!(result.1["exitCode"], 0);
    assert_eq!(
        std::fs::metadata(h.work.join("main.tex")).unwrap().ino(),
        inode
    );
    let (error, read) = h.read("read-after-exec").await;
    assert!(!error, "{read}");
    assert_eq!(read["text"], "shell-copy");
    let (error, patch) = h
        .execute(h.patch("patch-after-exec", "shell-copy", "patched-copy"))
        .await;
    assert!(!error, "{patch}");
    assert_eq!(patch["targets"][0]["status"], "applied");
    assert_eq!(
        std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
        "patched-copy\n"
    );
    assert!(h.tools.finalization_call().await.unwrap().is_some());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn file_only_patch_hands_off_once_and_old_handles_cannot_unlock_delivery_owner() {
    let h = Harness::new().await;
    assert!(!h.read("read-only").await.0);
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    let patch = h.patch("file-only", "Frozen manuscript", "file-only-copy");
    let original = h.execute(patch.clone()).await;
    assert!(!original.0, "{:?}", original.1);
    assert_eq!(original.1["targets"][0]["status"], "applied");
    assert!(h.tools.finalization_call().await.unwrap().is_some());
    let owned = h.source.workspace(&h.record).await.unwrap();
    let identity = owned
        .host
        .inspect_owned(|lease| Ok(lease.manifest().identity.clone()))
        .await
        .unwrap();
    assert!(h.tools.release_for_finalization().await.unwrap());
    let root = h
        .fixture
        .store
        .agent_workspace_directory()
        .canonicalize()
        .unwrap();
    let store = crate::agent::workspace::WorkspaceStore::open(&root, |_| Ok(())).unwrap();
    let delivery = store.restore(&identity).unwrap();
    assert!(matches!(
        h.tools
            .authorize_and_execute(h.call(
                "after-handoff",
                "research_read_file",
                json!({"path": h.path()}),
            ))
            .await,
        Err(RunError::ToolOutcomeUnknown | RunError::ToolOutcomeUnknownDiagnostic(_))
    ));
    assert!(matches!(
        h.tools
            .authorize_and_execute(h.patch("late-new", "file-only-copy", "must-not-write"))
            .await,
        Err(RunError::ToolOutcomeUnknown | RunError::ToolOutcomeUnknownDiagnostic(_))
    ));
    assert_eq!(
        h.execute(patch).await.1,
        original.1,
        "historical receipt replays without reopening a writer"
    );
    owned
        .host
        .inspect_owned(|lease| {
            lease
                .release_for_delivery()
                .map_err(|_| RunError::ToolRejected)
        })
        .await
        .unwrap();
    assert!(matches!(
        store.restore(&identity),
        Err(crate::agent::workspace::LeaseError::Busy)
    ));
    assert_eq!(
        delivery.read_text("main.tex").unwrap().as_deref(),
        Some("file-only-copy\n")
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
