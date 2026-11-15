//! Frozen model-visible schemas from the existing host builders, never execution.
use super::*;
use crate::agent::{composed_tools::RejectingToolExecutor, host_execution::HostTools};

pub(super) async fn prepare(
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    record: SubmissionRecord,
    models: AgentModelResolver,
    host: &Host,
    profile: &AgentExecutionProfile,
    namespace_tools: bool,
) -> Result<HostTools, Error> {
    let registered = if let Host::Manuscript { manuscript_id } = host {
        crate::agent::writing_submission::compaction_tool_catalog(
            store.clone(),
            record.clone(),
            sessions,
            models,
            *manuscript_id,
            namespace_tools,
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?
    } else {
        let mut definitions = match host {
            Host::Paper { .. } => crate::agent::reading_start::reading_host_tool_definitions(),
            Host::Note { .. } => crate::agent::note_tools::definitions(),
            Host::ReadingNote { .. } => vec![],
            Host::Manuscript { .. } => unreachable!(),
        };
        let skills = crate::agent::skills::application::tools(store.clone(), record.clone())
            .await
            .map_err(|_| Error::ModelUnavailable)?;
        definitions.extend(skills.definitions());
        #[cfg(unix)]
        if let Some(mcp) = crate::agent::child_runtime::mcp_runtime_for_conversation(
            &store,
            &sessions,
            record.conversation_id,
        )
        .await
        {
            let mcp = if namespace_tools {
                mcp.namespaced_view()
            } else {
                mcp
            };
            definitions.extend(mcp.definitions());
        }
        crate::agent::host_execution::registration_with_hooks(
            store.clone(),
            record.clone(),
            sessions,
            OwnedToolRegistration::new(definitions, Arc::new(RejectingToolExecutor)),
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?
    };
    #[cfg(target_os = "macos")]
    let tools = crate::agent::compaction_hook_spill::tool::registration(
        store.clone(),
        record.clone(),
        profile,
        registered.workspace.clone(),
        registered.tools,
    )
    .map_err(|_| Error::ModelUnavailable)?;
    #[cfg(not(target_os = "macos"))]
    let tools = {
        let _ = (&store, &record, profile);
        registered.tools
    };
    // Discard all prepared handlers: only schemas and the lazy hook workspace
    // belong to this task, even if a future caller tries to route a tool call.
    let tools = crate::agent::context_session::resource_tools::registration(
        store.clone(),
        record.clone(),
        tools,
    );
    let (definitions, _) = tools.into_parts();
    Ok(HostTools {
        tools: OwnedToolRegistration::new(definitions, Arc::new(RejectingToolExecutor)),
        workspace: registered.workspace,
    })
}
