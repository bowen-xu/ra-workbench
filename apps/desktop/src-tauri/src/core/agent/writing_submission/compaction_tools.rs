//! Reuse writing catalog construction; compaction receives no callable executor.
use super::*;
use crate::agent::{
    composed_tools::RejectingToolExecutor, host_execution::HostTools, model::AgentModelResolver,
    repository::SqliteConversationCatalog, service::AgentSessionService,
};
use research_agent_core::tool_registry::OwnedToolRegistration;
use std::sync::Arc;

pub(crate) async fn compaction_tool_catalog(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    models: AgentModelResolver,
    manuscript_id: Uuid,
    namespace_tools: bool,
) -> Result<HostTools, WritingSubmissionError> {
    let mcp = crate::agent::child_runtime::mcp_runtime_for_conversation(
        &store,
        &sessions,
        record.conversation_id,
    )
    .await
    .map(|runtime| {
        if namespace_tools {
            runtime.namespaced_view()
        } else {
            runtime
        }
    });
    let child = tools::ChildSpawnContext {
        store: store.clone(),
        catalog: SqliteConversationCatalog::from_app_store(&store),
        service: WritingSubmissionService::from_app_store(&store),
        models,
        sessions: sessions.clone(),
        parent_conversation_id: record.conversation_id,
        parent_turn_id: record.turn_id,
        context_snapshot_id: record.context_snapshot_id,
        manuscript_id,
        approved_servers: mcp
            .as_ref()
            .map(|runtime| runtime.approved_servers())
            .unwrap_or_default(),
    };
    let prepared =
        tools::for_submission_with_mcp_and_child(store, record, sessions, mcp, Some(child)).await?;
    Ok(HostTools {
        tools: OwnedToolRegistration::new(prepared.definitions(), Arc::new(RejectingToolExecutor)),
        workspace: prepared.hook_workspace(),
    })
}
