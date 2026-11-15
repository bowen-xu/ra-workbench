//! Sandboxed LaTeX compilation of an approved writing execution copy.
//!
//! This never calls the manuscript compiler: that path snapshots the live
//! manuscript and persists a production compile run. Here the immutable
//! submission binding owns a disposable work copy, an execution marker and the
//! process registry. Generated files are removed after a safely reaped process.
use super::*;
use crate::agent::writing_compile_contracts::{LatexToolchainInfo, LatexWorkspaceCompileRequest};
use crate::agent::{
    process::{
        OfflineSandboxPaths, ProcessLimits, ProcessPoll, ProcessRegistry, ProcessSpec, StopReason,
        prepare_offline_sandbox,
    },
    repository::{execution_workspaces, submissions},
    service::AgentSessionService,
    text_tools::file_workspace::FileWorkspaceSource,
    workspace::WorkspaceLease,
    workspace_compile::WritingWorkspaceCompileSource,
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    workspace::WorkspaceIdentity,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

pub(super) const COMPILE_WORKSPACE: &str = "research_compile_workspace";
const OUTPUT_LIMIT: usize = 12_000;
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    root_path: String,
    engine: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    stop_on_first_error: bool,
    #[serde(default)]
    from_scratch: bool,
}

#[derive(Clone)]
pub(super) struct CompileWorkspaceTools {
    store: AppStore,
    record: SubmissionRecord,
    binding: execution_workspaces::Binding,
    workspace: Arc<super::shell::Host>,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    scope: WorkspaceIdentity,
    manifest_hash: String,
    registry: Arc<ProcessRegistry>,
    compiler_source: Option<WritingWorkspaceCompileSource>,
    native_compiler: Option<crate::runtime_plugins::writing_projects::context::Binding>,
    #[cfg(test)]
    toolchain: Option<LatexToolchainInfo>,
    #[cfg(test)]
    before_output: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CompileWorkspaceTools {
    pub(super) async fn for_submission(
        store: AppStore,
        record: &SubmissionRecord,
        sessions: Arc<AgentSessionService>,
        workspace: Arc<super::shell::Host>,
    ) -> Result<Option<Self>, WritingSubmissionError> {
        let manuscript_id = manuscript_id(record)?;
        let lookup = store.clone();
        let conversation_id = record.conversation_id;
        let request_id = record.request_id;
        let turn_id = record.turn_id;
        let context_snapshot_id = record.context_snapshot_id;
        let approved = tokio::task::spawn_blocking(move || {
            let db = lookup
                .connection()
                .map_err(|_| WritingSubmissionError::Unavailable)?;
            let saved = match execution_workspaces::read(&db, conversation_id, request_id) {
                Ok(saved) => saved,
                Err(execution_workspaces::Error::Denied) => return Ok(None),
                Err(_) => return Err(WritingSubmissionError::Unavailable),
            };
            let Some((binding, manifest)) = saved else {
                return Ok(None);
            };
            if binding.turn_id != turn_id
                || binding.context_snapshot_id != context_snapshot_id
                || binding.manuscript_id != manuscript_id
            {
                return Err(SubmissionError::Conflict.into());
            }
            match execution_workspaces::authorize(&db, &binding, &manifest) {
                Ok(()) => {
                    let manifest_hash = binding.manifest_hash.clone();
                    Ok(Some((manifest.identity, manifest_hash, binding)))
                }
                Err(execution_workspaces::Error::Denied) => Ok(None),
                Err(_) => Err(WritingSubmissionError::Unavailable),
            }
        })
        .await
        .map_err(|_| WritingSubmissionError::Unavailable)??;
        let compiler_source = store.writing_workspace_compile_source();
        let native_compiler = compiler_source
            .is_none()
            .then(|| store.native_writing_context().cloned())
            .flatten();
        Ok(approved.map(|(scope, manifest_hash, binding)| Self {
            store,
            record: record.clone(),
            binding,
            workspace,
            receipt: WritingSubmissionReceipt {
                conversation_id,
                request_id,
                manuscript_id,
            },
            context_snapshot_id,
            scope,
            manifest_hash,
            registry: sessions.processes().clone(),
            compiler_source,
            native_compiler,
            #[cfg(test)]
            toolchain: None,
            #[cfg(test)]
            before_output: None,
        }))
    }

    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            namespace: None,
            name: COMPILE_WORKSPACE.into(),
            description: "Compile the approved offline writing copy with the installed LaTeX toolchain. The root path is relative to that copy; engine and compile flags are bounded. The live manuscript and its production compile history are never changed. Returns diagnostics and ephemeral PDF/SyncTeX availability after generated files are safely removed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "rootPath": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "engine": {"type": "string", "enum": ["pdflatex", "xelatex", "lualatex"]},
                    "draft": {"type": "boolean"},
                    "stopOnFirstError": {"type": "boolean"},
                    "fromScratch": {"type": "boolean"}
                },
                "required": ["rootPath", "engine"],
                "additionalProperties": false
            }),
        }]
    }

    fn parse(call: &ToolCall) -> Result<Request, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != COMPILE_WORKSPACE {
            return Err(RunError::ToolRejected);
        }
        let request: Request = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide rootPath, engine and optional compile flags using the declared types.",
        )?;
        if request.root_path.len() > 4096
            || !matches!(request.engine.as_str(), "pdflatex" | "xelatex" | "lualatex")
        {
            return Err(RunError::ToolRejected);
        }
        research_agent_core::workspace::validate_path(&request.root_path)
            .map_err(|_| RunError::ToolRejected)?;
        Ok(request)
    }

    async fn compile(&self, request: Request) -> Result<ToolOutput, RunError> {
        let workspace = self.workspace.workspace(&self.record).await?;
        if workspace.binding != self.binding {
            return Err(RunError::ToolRejected);
        }
        let owner = self.clone();
        let runtime = tokio::runtime::Handle::current();
        workspace
            .host
            .with_owned_lease_mut(move |lease| owner.run_owned(lease, request, runtime))
            .await?
    }

    fn authorize_owned(
        &self,
        db: &rusqlite::Connection,
        lease: &WorkspaceLease,
    ) -> Result<(), RunError> {
        lease
            .working_directory()
            .map_err(|_| RunError::ToolRejected)?;
        let saved =
            execution_workspaces::read(db, self.record.conversation_id, self.record.request_id)
                .map_err(|error| match error {
                    execution_workspaces::Error::Storage => RunError::EventSink,
                    _ => RunError::ToolRejected,
                })?
                .ok_or(RunError::ToolRejected)?;
        if saved.0 != self.binding
            || &saved.1 != lease.manifest()
            || saved.1.identity != self.scope
            || submissions::read(db, self.record.conversation_id, self.record.request_id)
                .map_err(|_| RunError::EventSink)?
                .as_ref()
                != Some(&self.record)
        {
            return Err(RunError::ToolRejected);
        }
        execution_workspaces::authorize(db, &self.binding, lease.manifest()).map_err(|error| {
            match error {
                execution_workspaces::Error::Storage => RunError::EventSink,
                _ => RunError::ToolRejected,
            }
        })?;
        lease.changes().map_err(|_| RunError::ToolRejected)?;
        Ok(())
    }

    fn run_owned(
        &self,
        lease: &mut WorkspaceLease,
        request: Request,
        runtime: tokio::runtime::Handle,
    ) -> Result<ToolOutput, RunError> {
        {
            let db = self.store.connection().map_err(|_| RunError::EventSink)?;
            self.authorize_owned(&db, lease)?;
        }
        let compiler = compiler::Compiler::select(self, runtime.clone())?;
        // Detection may consult SQL and launch bounded trusted version probes.
        // Never retain the connection guard across it or the process wait.
        #[cfg(test)]
        let toolchain = match &self.toolchain {
            Some(value) => value.clone(),
            None => compiler.detect(&self.store)?,
        };
        #[cfg(not(test))]
        let toolchain = compiler.detect(&self.store)?;
        let result_id = Uuid::new_v4();
        let root_path = request.root_path.clone();
        let prepared = prepare_launch(self, lease, request, result_id, toolchain, &compiler)?;
        // One actual blocking worker retains the shared mutex across launch,
        // supervision, result snapshot and cleanup. Process retention itself
        // clears its marker without acquiring this mutex.
        let outcome = runtime.block_on(async {
            loop {
                match self
                    .registry
                    .poll(&self.scope, prepared.process_id, Duration::from_secs(60))
                    .await
                    .map_err(|_| RunError::EventSink)?
                {
                    ProcessPoll::Running(_) => continue,
                    ProcessPoll::Finished(outcome) => return Ok::<_, RunError>(outcome),
                }
            }
        })?;
        if !outcome.drained || outcome.resource_error.is_some() {
            return Err(RunError::EventSink);
        }
        let forgotten = self
            .registry
            .forget(&self.scope, prepared.process_id)
            .map_err(|_| RunError::EventSink);
        #[cfg(test)]
        if let Some(gate) = &self.before_output {
            gate();
        }
        let output_directory = prepared.output_directory;
        let result = (|| {
            forgotten?;
            let raw_output = output::render_output(&outcome.stdout, &outcome.stderr);
            let diagnostics = compiler.prepare_diagnostics(&raw_output)?;
            let db = self.store.connection().map_err(|_| RunError::EventSink)?;
            self.authorize_owned(&db, lease)?;
            drop(db);
            finish_output(
                output_directory.clone(),
                root_path,
                &outcome,
                result_id,
                self.store.clone(),
                self.receipt,
                self.context_snapshot_id,
                &self.scope,
                &self.manifest_hash,
                lease,
                diagnostics,
                raw_output,
            )
        })();
        self.cleanup_output(&output_directory, result_id)?;
        result
    }

    fn cleanup_output(&self, output_directory: &Path, result_id: Uuid) -> Result<(), RunError> {
        if remove_ephemeral_directory(output_directory).is_err() {
            let diagnostic = research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(
                json!({"code":"WORKSPACE_COMPILE_CLEANUP_REQUIRED","conversationId":self.record.conversation_id,"requestId":self.record.request_id,"turnId":self.record.turn_id,"contextSnapshotId":self.record.context_snapshot_id,"manuscriptId":self.receipt.manuscript_id,"leaseId":self.scope.lease_id,"resultId":result_id,"phase":"compile-output-cleanup","safeCause":"ephemeral_output_cleanup_unconfirmed"}),
            );
            return Err(match diagnostic {
                Ok(detail) => RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail)),
                Err(_) => RunError::ToolOutcomeUnknown,
            });
        }
        Ok(())
    }
}

fn detect_toolchain(
    store: &AppStore,
    source: WritingWorkspaceCompileSource,
) -> Result<LatexToolchainInfo, RunError> {
    let checked_at = {
        let connection = store.connection().map_err(|_| RunError::ToolUnavailable)?;
        connection
            .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get(0)
            })
            .map_err(|_| RunError::ToolUnavailable)?
    };
    Ok((source.detect)(
        checked_at,
        &store.writing_cache_directory().join("toolchain-home"),
    ))
}

impl ToolExecutor for CompileWorkspaceTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        Self::parse(call).map(|_| ())
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let request = Self::parse(&call)?;
            self.compile(request).await
        })
    }
}

pub(super) use crate::agent::workspace::sandbox::protected_paths;
#[cfg(test)]
use crate::agent::workspace::sandbox::protected_paths_for_roots;

fn manuscript_id(record: &SubmissionRecord) -> Result<Uuid, WritingSubmissionError> {
    record
        .input
        .context
        .iter()
        .find_map(|source| {
            source
                .anchor
                .as_deref()
                .and_then(|anchor| serde_json::from_str::<serde_json::Value>(anchor).ok())
                .and_then(|anchor| {
                    anchor
                        .get("manuscriptId")?
                        .as_str()
                        .and_then(|id| Uuid::parse_str(id).ok())
                })
        })
        .ok_or(SubmissionError::Corrupt.into())
}

#[path = "compile_workspace/compiler.rs"]
mod compiler;
#[path = "compile_workspace/launch.rs"]
mod launch;
#[path = "compile_workspace/output.rs"]
mod output;
use launch::prepare_launch;
#[cfg(test)]
use output::tail;
use output::{finish_output, remove_ephemeral_directory};
#[cfg(test)]
#[path = "compile_workspace/owned_tests.rs"]
mod owned_tests;
#[cfg(test)]
#[path = "compile_workspace/tests.rs"]
mod tests;
