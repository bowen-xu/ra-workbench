//! Compose registered native tools without changing frozen source-tool ownership.
use super::*;
use crate::agent::{service::AgentSessionService, skills::tools::SkillTools};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
    subtasks::SpawnRequest,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;
#[path = "tools/compile.rs"]
mod compile;
#[cfg(target_os = "macos")]
#[path = "tools/compile_workspace.rs"]
mod compile_workspace;
#[path = "tools/git.rs"]
mod git;
#[cfg(target_os = "macos")]
#[path = "tools/owned_workspace.rs"]
mod owned_workspace;
#[cfg(unix)]
#[path = "tools/patch.rs"]
mod patch;
#[cfg(target_os = "macos")]
#[path = "tools/shell.rs"]
mod shell;
#[path = "tools/workspace.rs"]
mod workspace;

pub(super) const SPAWN_SUBTASK: &str = "research_spawn_subtask";

#[derive(Clone)]
pub(super) struct ChildSpawnContext {
    pub store: AppStore,
    pub catalog: crate::agent::repository::SqliteConversationCatalog,
    pub service: crate::agent::writing_submission::WritingSubmissionService,
    pub models: crate::agent::model::AgentModelResolver,
    pub sessions: Arc<AgentSessionService>,
    pub parent_conversation_id: Uuid,
    pub parent_turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub manuscript_id: Uuid,
    pub approved_servers: HashSet<String>,
}

struct ChildSpawnTool {
    context: ChildSpawnContext,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpawnArgs {
    prompt: String,
}

impl ChildSpawnTool {
    fn definition() -> ToolDefinition {
        ToolDefinition {
            namespace: None,
            name: SPAWN_SUBTASK.into(),
            description: "Delegate a bounded, manuscript-scoped research task to a separate child conversation. The child inherits the current frozen context and can never expand the parent budget or approved tools. Use only for work that can proceed independently; the returned child conversation remains separate from this history.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"prompt": {"type": "string", "minLength": 1, "maxLength": 16384}},
                "required": ["prompt"],
                "additionalProperties": false
            }),
        }
    }

    fn parse(
        call: &ToolCall,
        context_snapshot_id: Uuid,
        approved_servers: &HashSet<String>,
    ) -> Result<(SpawnArgs, SpawnRequest), research_agent_core::runner::RunError> {
        if call.name != SPAWN_SUBTASK || call.arguments.to_string().len() > 32 * 1024 {
            return Err(research_agent_core::runner::RunError::ToolRejected);
        }
        let args: SpawnArgs = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide the required prompt string; no extra fields are accepted.",
        )?;
        let request = SpawnRequest {
            prompt: args.prompt.clone(),
            context_snapshot_id,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: approved_servers.clone(),
        };
        request
            .validate()
            .map_err(|_| research_agent_core::runner::RunError::ToolRejected)?;
        Ok((args, request))
    }
}

impl ToolExecutor for ChildSpawnTool {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        (call.name == SPAWN_SUBTASK).then(|| {
            super::super::child_creation::intent(
                super::super::child_creation::writing_tool_child_id(
                    self.context.parent_turn_id,
                    &call.call_id,
                ),
                research_agent_core::conversation::ConversationBinding {
                    kind: research_agent_core::conversation::BindingKind::Manuscript,
                    resource_id: self.context.manuscript_id.to_string(),
                },
                super::super::child_creation::Host::Writing,
            )
        })
    }
    fn validate(&self, call: &ToolCall) -> Result<(), research_agent_core::runner::RunError> {
        Self::parse(
            call,
            self.context.context_snapshot_id,
            &self.context.approved_servers,
        )
        .map(|_| ())
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let (args, _request) = Self::parse(
                &call,
                self.context.context_snapshot_id,
                &self.context.approved_servers,
            )?;
            let child_id = super::super::child_creation::writing_tool_child_id(
                self.context.parent_turn_id,
                &call.call_id,
            );
            let response = super::super::writing_child_commands::spawn_writing_child_with_id(
                self.context.store.clone(),
                self.context.catalog.clone(),
                self.context.service.clone(),
                self.context.models.clone(),
                self.context.sessions.clone(),
                super::super::writing_child_commands::Request {
                    parent_conversation_id: self.context.parent_conversation_id,
                    parent_turn_id: self.context.parent_turn_id,
                    manuscript_id: self.context.manuscript_id,
                    approved_servers: self.context.approved_servers.clone(),
                    prompt: args.prompt,
                },
                child_id,
            )
            .await;
            match response {
                Ok(response) => Ok(ToolOutput {
                    content: serde_json::to_string(&response)
                        .map_err(|_| research_agent_core::runner::RunError::ToolUnavailable)?,
                    is_error: false,
                }),
                Err(error) => writing_child_failure(&self.context, child_id, &error),
            }
        })
    }
}

fn writing_child_failure(
    context: &ChildSpawnContext,
    child_id: Uuid,
    error: &super::super::writing_child_commands::Error,
) -> Result<ToolOutput, research_agent_core::runner::RunError> {
    use super::super::{
        catalog_commands::CatalogMutationCommandError, writing_child_commands::Error,
    };
    use research_agent_core::{
        conversation::{BindingKind, CatalogDataFailureKind},
        runner::{RunError, ToolOutcomeDiagnostic},
    };
    if let Error::ArchiveMutation(failure) = error {
        return Err(super::super::child_archive::unresolved_tool_error(
            failure, child_id,
        ));
    }
    if let Error::ChildSubmissionUnconfirmed(failure) = error {
        return Err(super::super::child_submission::tool_error(
            failure,
            child_id,
            context.parent_conversation_id,
            context.parent_turn_id,
            context.context_snapshot_id,
        ));
    }
    if let Error::ChildTrackingUnconfirmed(failure) = error {
        return Err(failure.tool_error(
            child_id,
            context.parent_conversation_id,
            context.parent_turn_id,
            context.context_snapshot_id,
        ));
    }
    if let Error::CatalogMutation(failure) = error
        && let CatalogMutationCommandError::Data(data) = &failure.reason
        && matches!(
            data.kind,
            CatalogDataFailureKind::Unknown
                | CatalogDataFailureKind::Partial
                | CatalogDataFailureKind::ExternalUnknown
                | CatalogDataFailureKind::RecoveryRequired
        )
    {
        let expected = super::super::child_creation::intent(
            child_id,
            research_agent_core::conversation::ConversationBinding {
                kind: BindingKind::Manuscript,
                resource_id: context.manuscript_id.to_string(),
            },
            super::super::child_creation::Host::Writing,
        );
        if failure.creation != (&expected).into()
            || data.diagnostic.operation_id != expected.operation_id
        {
            return Err(RunError::ToolOutcomeUnknown);
        }
        let detail = json!({
            "code":"CHILD_CREATION_UNCONFIRMED",
            "phase": data.diagnostic.phase,
            "safeCause": data.diagnostic.safe_cause,
            "conversationId": context.parent_conversation_id,
            "turnId": context.parent_turn_id,
            "contextSnapshotId": context.context_snapshot_id,
            "manuscriptId": context.manuscript_id,
            "requestId": expected.operation_id,
            "resultId": child_id,
            "sqliteCode": data.diagnostic.sqlite_code,
            "sqliteExtendedCode": data.diagnostic.sqlite_extended_code,
        });
        return Err(ToolOutcomeDiagnostic::try_from_value(detail)
            .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
            .unwrap_or(RunError::ToolOutcomeUnknown));
    }
    Ok(ToolOutput {
        content: format!(
            "child task could not be started ({}) ; no child history was exposed",
            serde_json::to_string(error).unwrap_or_else(|_| "childSpawnFailed".into())
        ),
        is_error: true,
    })
}

#[cfg(test)]
#[path = "tools/child_tests.rs"]
mod child_tests;

pub(super) struct WritingTools {
    skills: SkillTools,
    compile: compile::CompileTools,
    git: Option<git::GitTools>,
    #[cfg(target_os = "macos")]
    compile_workspace: Option<compile_workspace::CompileWorkspaceTools>,
    workspace: workspace::WorkspaceTools,
    #[cfg(target_os = "macos")]
    shell: Option<crate::agent::shell_tools::ShellTools>,
    #[cfg(unix)]
    patch: Option<patch::PatchTools>,
    #[cfg(unix)]
    mcp_runtime: Option<Arc<dyn crate::agent::mcp_runtime::McpToolProvider>>,
    child_spawn: Option<ChildSpawnTool>,
}
#[allow(dead_code, reason = "kept for shell/tool fixture construction")]
pub(super) async fn for_submission(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
) -> Result<WritingTools, WritingSubmissionError> {
    #[cfg(unix)]
    let mcp_runtime = super::super::child_runtime::mcp_runtime_for_conversation(
        &store,
        &sessions,
        record.conversation_id,
    )
    .await;
    #[cfg(unix)]
    return for_submission_with_mcp(store, record, sessions.clone(), mcp_runtime).await;
    #[cfg(not(unix))]
    {
        let skills = super::skill_tools::for_submission(store.clone(), record.clone()).await?;
        let compile = compile::CompileTools::new(store.clone(), &record)?;
        let git = if crate::agent::context_session::permissions::writing_paths(&record).is_some() {
            None
        } else {
            git::GitTools::for_submission(&store, &record).await
        };
        let workspace = workspace::WorkspaceTools::new(store.clone(), &record)?;
        #[cfg(target_os = "macos")]
        let (workspace, shell, compile_workspace) =
            owned_workspace::compose(store.clone(), &record, sessions.clone(), workspace).await?;
        #[cfg(unix)]
        let patch = patch::PatchTools::for_submission(store.clone(), &record).await?;
        #[cfg(not(target_os = "macos"))]
        let _ = sessions;
        Ok(WritingTools {
            skills,
            compile,
            git,
            #[cfg(target_os = "macos")]
            compile_workspace,
            workspace,
            #[cfg(target_os = "macos")]
            shell,
            #[cfg(unix)]
            patch,
            child_spawn: None,
        })
    }
}
#[cfg(unix)]
#[allow(dead_code, reason = "kept for platform-specific tool fixtures")]
pub(super) async fn for_submission_with_mcp(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    mcp_runtime: Option<Arc<dyn crate::agent::mcp_runtime::McpToolProvider>>,
) -> Result<WritingTools, WritingSubmissionError> {
    for_submission_with_mcp_and_child(store, record, sessions, mcp_runtime, None).await
}

pub(super) async fn for_submission_with_mcp_and_child(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    mcp_runtime: Option<Arc<dyn crate::agent::mcp_runtime::McpToolProvider>>,
    child_context: Option<ChildSpawnContext>,
) -> Result<WritingTools, WritingSubmissionError> {
    let skills = super::skill_tools::for_submission(store.clone(), record.clone()).await?;
    let compile = compile::CompileTools::new(store.clone(), &record)?;
    let scoped = crate::agent::context_session::permissions::writing_paths(&record).is_some();
    let git = if scoped {
        None
    } else {
        git::GitTools::for_submission(&store, &record).await
    };
    let workspace = workspace::WorkspaceTools::new(store.clone(), &record)?;
    #[cfg(target_os = "macos")]
    let (workspace, shell, compile_workspace) =
        owned_workspace::compose(store.clone(), &record, sessions.clone(), workspace).await?;
    #[cfg(unix)]
    let patch = patch::PatchTools::for_submission(store.clone(), &record).await?;
    #[cfg(not(target_os = "macos"))]
    let _ = sessions;
    Ok(WritingTools {
        skills,
        compile,
        git,
        #[cfg(target_os = "macos")]
        compile_workspace,
        workspace,
        #[cfg(target_os = "macos")]
        shell,
        #[cfg(unix)]
        patch,
        mcp_runtime,
        // Subtask recovery is not complete; retain the context types for old
        // histories but never expose or execute child creation for new turns.
        child_spawn: {
            let _ = child_context;
            None
        },
    })
}
impl WritingTools {
    #[cfg(target_os = "macos")]
    pub(super) fn file_workspace(
        &self,
    ) -> crate::agent::text_tools::file_workspace::FileWorkspaceHandle {
        self.workspace.file_workspace()
    }
    pub(super) fn hook_workspace(
        &self,
    ) -> crate::agent::compaction_hook_factory::HookWorkspaceHandle {
        self.workspace.hook_workspace()
    }
    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self.skills.definitions();
        definitions.extend(self.compile.definitions());
        if let Some(git) = &self.git {
            definitions.extend(git.definitions());
        }
        #[cfg(target_os = "macos")]
        if let Some(compile_workspace) = &self.compile_workspace {
            definitions.extend(compile_workspace.definitions());
        }
        definitions.extend(self.workspace.definitions());
        #[cfg(unix)]
        if let Some(patch) = &self.patch {
            definitions.extend(patch.definitions());
        }
        if self.child_spawn.is_some() {
            definitions.push(ChildSpawnTool::definition());
        }
        #[cfg(unix)]
        if let Some(runtime) = &self.mcp_runtime {
            definitions.extend(runtime.definitions());
        }
        #[cfg(target_os = "macos")]
        let definitions = if let Some(shell) = &self.shell {
            let mut definitions = definitions;
            definitions.extend(shell.definitions());
            definitions
        } else {
            definitions
        };
        definitions
    }
    fn executor(&self, call: &ToolCall) -> &dyn ToolExecutor {
        if call.namespace.is_none() && call.name == SPAWN_SUBTASK {
            return &crate::agent::composed_tools::RejectingToolExecutor;
        }
        if call.namespace.is_some() {
            #[cfg(unix)]
            if let Some(runtime) = &self.mcp_runtime
                && runtime.resolve_qualified_name(call.namespace.as_deref(), &call.name)
                    == ToolNameResolution::Registered
            {
                return runtime.as_ref();
            }
            return &crate::agent::composed_tools::RejectingToolExecutor;
        }
        #[cfg(unix)]
        if let Some(runtime) = &self.mcp_runtime
            && runtime.resolve_name(&call.name) == ToolNameResolution::Registered
        {
            return runtime.as_ref();
        }
        if matches!(
            call.name.as_str(),
            compile::READ_DIAGNOSTICS | compile::FORWARD_SYNCTEX
        ) {
            return &self.compile;
        }
        if matches!(call.name.as_str(), git::STATUS | git::LOG | git::DIFF)
            && let Some(git) = &self.git
        {
            return git;
        }
        #[cfg(target_os = "macos")]
        if call.name == compile_workspace::COMPILE_WORKSPACE
            && let Some(compile_workspace) = &self.compile_workspace
        {
            return compile_workspace;
        }
        if matches!(
            call.name.as_str(),
            workspace::REVIEW_CHANGES | workspace::READ_CHANGE
        ) {
            return &self.workspace;
        }
        #[cfg(unix)]
        if call.name == patch::APPLY_PATCH
            && let Some(patch) = &self.patch
        {
            return patch;
        }
        #[cfg(not(target_os = "macos"))]
        let _ = call;
        #[cfg(target_os = "macos")]
        if matches!(
            call.name.as_str(),
            crate::agent::shell_tools::EXEC | crate::agent::shell_tools::PROCESS
        ) && let Some(shell) = &self.shell
        {
            return shell;
        }
        &self.skills
    }
}
impl ToolExecutor for WritingTools {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        self.executor(call).child_creation_intent(call)
    }
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        // Definitions are composed from this instance's frozen handlers, not
        // the model request's advertised catalog. Constructing them performs
        // no I/O and never checks or changes authorization.
        if self
            .definitions()
            .iter()
            .any(|tool| tool.namespace.is_none() && tool.name == name)
        {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        if namespace.is_none() {
            return self.resolve_name(name);
        }
        #[cfg(unix)]
        if let Some(runtime) = &self.mcp_runtime {
            return runtime.resolve_qualified_name(namespace, name);
        }
        ToolNameResolution::NotFound
    }
    fn cancel_pending(&self, call: ToolCall) -> RunFuture<'_, ()> {
        self.executor(&call).cancel_pending(call)
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        self.executor(call).supports_parallel(call)
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        self.executor(call).cancellation_safe_to_drop(call)
    }
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        self.executor(call).requires_result_receipt(call)
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        self.workspace.finalization_call()
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        self.workspace.release_for_finalization()
    }
    fn validate(&self, call: &ToolCall) -> Result<(), research_agent_core::runner::RunError> {
        self.executor(call).validate(call)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.executor(&call).authorize_and_execute(call)
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        self.executor(&call).result_committed(call)
    }
}
