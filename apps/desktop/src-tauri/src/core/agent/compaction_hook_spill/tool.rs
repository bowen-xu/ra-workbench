//! A read-only tool in the existing owned registration. No model-provided scope,
//! native paths, filesystem grants, lease acquisition or alternate task runner.
use super::*;
use crate::{
    agent::{
        compaction_hook_config::native::HookWorkspaceSource, composed_tools::ComposedToolExecutor,
        execution_profile::AgentExecutionProfile, repository::hook_spills,
    },
    storage::AppStore,
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    submission::SubmissionRecord,
    tool_registry::OwnedToolRegistration,
};
use serde::Deserialize;
use uuid::Uuid;

pub(crate) const NAME: &str = "read_hook_output";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arguments {
    artifact_id: Uuid,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}
struct ReaderTool {
    store: AppStore,
    record: Arc<SubmissionRecord>,
    workspace: Arc<dyn HookWorkspaceSource>,
    max_bytes: usize,
}
impl ReaderTool {
    fn arguments(&self, call: &ToolCall) -> Result<Arguments, RunError> {
        if call.name != NAME {
            return Err(RunError::ToolRejected);
        }
        let args: Arguments = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide artifactId and optional offset and limit with the declared types.",
        )?;
        if args.artifact_id.is_nil()
            || args
                .limit
                .is_some_and(|n| !(4..=self.max_bytes).contains(&n))
        {
            return Err(RunError::ToolRejected);
        }
        Ok(args)
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            namespace: None,
            name: NAME.into(),
            description: "Read preserved full hook output referenced by hook-artifact://UUID. Use the UUID as artifactId. Returns a UTF-8 byte slice, sha256, totalBytes and nextOffset; use nextOffset to continue. Only completed context in this conversation and exact model is readable; locator text does not grant access. This does not execute a hook or read arbitrary files.".into(),
            parameters: serde_json::json!({"type":"object","additionalProperties":false,"required":["artifactId"],"properties":{
                "artifactId":{"type":"string","format":"uuid"},
                "offset":{"type":"integer","minimum":0,"description":"UTF-8 byte boundary; default 0"},
                "limit":{"type":"integer","minimum":4,"maximum":self.max_bytes,"description":"Maximum UTF-8 bytes; default up to 4096 within the frozen profile ceiling"}
            }}),
        }
    }
}
impl ToolExecutor for ReaderTool {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.arguments(call).map(|_| ())
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let args = self.arguments(&call)?;
            let store = self.store.clone();
            let record = self.record.clone();
            // The cutoff is native journal state at this owned invocation,
            // including the just-completed SessionStart. It is never an argument.
            let through = tokio::task::spawn_blocking(move || {
                let db = store.connection().map_err(|_| RunError::EventSink)?;
                hook_spills::invocation_cutoff(&db, record.conversation_id, record.turn_id)
                    .map_err(|_| RunError::ToolRejected)
            })
            .await
            .map_err(|_| RunError::EventSink)??;
            let workspace = self.workspace.clone();
            let slice = native::NativeHookSpillStore::new(self.store.clone())
                .read(
                    HookArtifactRead {
                        submission: self.record.clone(),
                        through,
                        artifact_id: args.artifact_id,
                        offset: args.offset,
                        limit: args.limit.unwrap_or(self.max_bytes.min(4096)),
                    },
                    Arc::new(move |record| workspace.authorize_artifact_read(record)),
                )
                .await
                .map_err(|_| RunError::ToolRejected)?;
            let content = serde_json::json!({"text":slice.text,"totalBytes":slice.total_bytes,"nextOffset":slice.next_offset,"sha256":slice.sha256}).to_string();
            Ok(ToolOutput {
                content,
                is_error: false,
            })
        })
    }
}

pub(crate) fn registration(
    store: AppStore,
    record: SubmissionRecord,
    profile: &AgentExecutionProfile,
    workspace: Option<Arc<dyn HookWorkspaceSource>>,
    existing: OwnedToolRegistration,
) -> Result<OwnedToolRegistration, RunError> {
    if profile.model != record.input.model {
        return Err(RunError::ToolRejected);
    }
    let Some(workspace) = workspace else {
        return Ok(existing);
    };
    if !profile.capabilities.tools {
        return Ok(existing);
    }
    // JSON text can expand to six bytes per source byte. Reserve metadata space
    // so the existing runner's tool output cap also bounds this response.
    let max_bytes = (profile.limits.tool_output_bytes.saturating_sub(512) / 6).min(65536);
    if max_bytes < 4 {
        return Err(RunError::ToolRejected);
    }
    let tool = ReaderTool {
        store,
        record: Arc::new(record),
        workspace,
        max_bytes,
    };
    let definitions = vec![tool.definition()];
    let (mut current, executor) = existing.into_parts();
    if current.iter().any(|d| d.name == NAME) {
        return Err(RunError::ToolRejected);
    }
    let composed = ComposedToolExecutor::new(
        executor,
        current.clone(),
        Some(Arc::new(tool)),
        definitions.clone(),
    );
    current.extend(definitions);
    Ok(OwnedToolRegistration::new(current, Arc::new(composed)))
}

#[cfg(test)]
#[path = "tool/tests.rs"]
mod tests;
