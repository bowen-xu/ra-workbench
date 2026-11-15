//! Shared native ownership helpers for already-admitted child conversations.
//!
//! Host adapters create their domain-specific frozen submission, then reuse
//! this module for budget validation, durable scheduling metadata, parent
//! cancellation lineage, and terminal reconciliation.

use super::{
    repository::SqliteConversationCatalog,
    service::{AgentSessionService, OwnedTurn},
    tasks::TaskCompletion,
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{ConversationBinding, ConversationCatalog, ConversationId},
    runner::RunLimits,
    submission::SubmissionRecord,
    subtasks::SubtaskSpec,
};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

#[path = "child_runtime/notifications.rs"]
mod notifications;

#[cfg(unix)]
fn runtime_identities_match(
    installed: &[(String, String, String)],
    configured: &mut [(String, String, String)],
) -> bool {
    let mut installed = installed.to_vec();
    installed.sort();
    configured.sort();
    installed == configured
}

pub(crate) struct SpecInputs {
    pub parent_conversation_id: Uuid,
    pub child_conversation_id: Uuid,
    pub parent_context_snapshot_id: Uuid,
    pub child_context_snapshot_id: Uuid,
    pub parent_depth: u8,
    pub parent_model_calls: usize,
    pub parent_tool_calls: usize,
    pub child_model_calls: usize,
    pub child_tool_calls: usize,
    pub parent_approved_servers: HashSet<String>,
    pub child_approved_servers: HashSet<String>,
}

pub(crate) struct ParentBounds {
    pub depth: u8,
    pub model_calls: usize,
    pub tool_calls: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingFailure {
    pub child_conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
}

impl TrackingFailure {
    pub(crate) fn tool_error(
        &self,
        expected_child: Uuid,
        parent: Uuid,
        parent_turn: Uuid,
        parent_context: Uuid,
    ) -> research_agent_core::runner::RunError {
        use research_agent_core::runner::{RunError, ToolOutcomeDiagnostic};
        if self.child_conversation_id != expected_child {
            return RunError::ToolOutcomeUnknown;
        }
        let detail = serde_json::json!({
            "code": "CHILD_TRACKING_UNCONFIRMED",
            "phase": "child-tracking",
            "safeCause": "child_cancellation_unconfirmed",
            "conversationId": parent,
            "requestId": self.request_id,
            "turnId": parent_turn,
            "contextSnapshotId": parent_context,
            "childConversationId": self.child_conversation_id,
            "childTurnId": self.turn_id,
            "childContextSnapshotId": self.context_snapshot_id,
        });
        ToolOutcomeDiagnostic::try_from_value(detail)
            .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
            .unwrap_or(RunError::ToolOutcomeUnknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumePreparationError {
    InvalidIdentity,
    ParentBinding,
    ChildBinding,
    Task,
    Submission,
    ContextSnapshot,
}

pub(crate) async fn resolve_parent_bounds(
    store: &AppStore,
    conversation_id: Uuid,
    context_snapshot_id: Uuid,
    root_limits: RunLimits,
) -> Result<ParentBounds, ()> {
    let db = store.shared_agent_connection();
    let record = tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|_| ())?;
        super::repository::subtasks::find_by_child(&connection, conversation_id).map_err(|_| ())
    })
    .await
    .map_err(|_| ())??;
    match record {
        Some(record)
            if record.status == super::repository::subtasks::Status::Running
                && record.child_context_snapshot_id == context_snapshot_id =>
        {
            Ok(ParentBounds {
                depth: record.depth,
                model_calls: record.model_calls as usize,
                tool_calls: record.tool_calls as usize,
            })
        }
        Some(_) => Err(()),
        None => Ok(ParentBounds {
            depth: 0,
            // Delegation has an explicit application-owned allowance even when
            // its ordinary parent follows the unbounded Codex continuation loop.
            model_calls: root_limits.model_calls.map_or(16, |max| max.min(16)),
            tool_calls: root_limits.tool_calls.map_or(64, |max| max.min(64)),
        }),
    }
}

/// Validate the shared recovery boundary and return the frozen child
/// submission. Host adapters call this before selecting their existing start
/// path; it never reads mutable host state or starts execution.
pub(crate) async fn prepare_resume(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    parent: Uuid,
    child: Uuid,
    binding: ConversationBinding,
) -> Result<(super::repository::subtasks::Record, SubmissionRecord), ResumePreparationError> {
    if [parent, child].into_iter().any(|id| id.is_nil()) {
        return Err(ResumePreparationError::InvalidIdentity);
    }
    catalog
        .read(ConversationId::from_uuid(parent), Some(binding.clone()))
        .await
        .map_err(|_| ResumePreparationError::ParentBinding)?;
    catalog
        .read(ConversationId::from_uuid(child), Some(binding))
        .await
        .map_err(|_| ResumePreparationError::ChildBinding)?;
    let db = store.shared_agent_connection();
    let task = {
        let db = db.lock().map_err(|_| ResumePreparationError::Task)?;
        super::repository::subtasks::find_by_child(&db, child)
            .map_err(|_| ResumePreparationError::Task)?
            .ok_or(ResumePreparationError::Task)?
    };
    if task.parent_conversation_id != parent
        || task.status != super::repository::subtasks::Status::RecoveryRequired
    {
        return Err(ResumePreparationError::Task);
    }
    let submissions = super::repository::SqliteSubmissionStore::for_app(store);
    let record = submissions
        .latest_for_conversation(child)
        .await
        .map_err(|_| ResumePreparationError::Submission)?;
    if record.context_snapshot_id != task.child_context_snapshot_id {
        return Err(ResumePreparationError::ContextSnapshot);
    }
    Ok((task, record))
}

pub(crate) async fn claim_resume(store: &AppStore, child: Uuid) -> Result<bool, ()> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        notifications::write(&store, (), |db| {
            let parent = notifications::parent(db, child).map_err(|_| ())?;
            let changed =
                super::repository::subtasks::claim_for_resume(db, child).map_err(|_| ())?;
            Ok((changed, parent.filter(|_| changed)))
        })
    })
    .await
    .map_err(|_| ())?
}

/// Clone the latest frozen child submission into a new retry turn while the
/// durable child claim is held. The host callback copies any business scope
/// row that is keyed by submission identity (paper is the current example).
/// The old interrupted turn and its request identity remain immutable.
pub(crate) async fn create_retry_submission<F>(
    store: &AppStore,
    child: Uuid,
    previous: &SubmissionRecord,
    copy_scope: F,
) -> Result<SubmissionRecord, ()>
where
    F: FnOnce(&rusqlite::Transaction<'_>, &SubmissionRecord) -> Result<(), ()> + Send + 'static,
{
    if child != previous.conversation_id {
        return Err(());
    }
    let store = store.clone();
    let source_request = previous.request_id;
    let source_context = previous.context_snapshot_id;
    let retry_request = Uuid::new_v4();
    tokio::task::spawn_blocking(move || {
        notifications::write(&store, (), |db| {
            let parent = notifications::parent(db, child)
                .map_err(|_| ())?
                .ok_or(())?;
            let tx = db.transaction().map_err(|_| ())?;
            let retry = super::repository::submissions::retry_frozen_in(
                &tx,
                child,
                source_request,
                retry_request,
            )
            .map_err(|_| ())?;
            copy_scope(&tx, &retry)?;
            if !super::repository::subtasks::bind_retry_context(
                &tx,
                child,
                source_context,
                retry.context_snapshot_id,
            )
            .map_err(|_| ())?
            {
                return Err(());
            }
            tx.commit().map_err(|_| ())?;
            Ok((retry, Some(parent)))
        })
    })
    .await
    .map_err(|_| ())?
}

pub(crate) async fn release_resume(store: &AppStore, child: Uuid) {
    let store = store.clone();
    let _ = tokio::task::spawn_blocking(move || {
        notifications::write(&store, (), |db| {
            let parent = notifications::parent(db, child).map_err(|_| ())?;
            let changed =
                super::repository::subtasks::release_resume_claim(db, child).map_err(|_| ())?;
            Ok(((), parent.filter(|_| changed)))
        })
    })
    .await;
}

pub(crate) fn bounded_specs(input: SpecInputs) -> (SubtaskSpec, SubtaskSpec) {
    let parent = SubtaskSpec {
        parent_conversation_id: input.parent_conversation_id,
        // The parent spec describes the parent owner itself. The validator
        // compares this identity with the child spec's parent identity.
        child_conversation_id: input.parent_conversation_id,
        parent_context_snapshot_id: input.parent_context_snapshot_id,
        child_context_snapshot_id: input.parent_context_snapshot_id,
        depth: input.parent_depth,
        model_calls: input.parent_model_calls,
        tool_calls: input.parent_tool_calls,
        approved_servers: input.parent_approved_servers,
    };
    let child = SubtaskSpec {
        parent_conversation_id: input.parent_conversation_id,
        child_conversation_id: input.child_conversation_id,
        parent_context_snapshot_id: input.parent_context_snapshot_id,
        child_context_snapshot_id: input.child_context_snapshot_id,
        depth: input.parent_depth.saturating_add(1),
        model_calls: input.child_model_calls.min(parent.model_calls),
        tool_calls: input.child_tool_calls.min(parent.tool_calls),
        approved_servers: input.child_approved_servers,
    };
    (parent, child)
}

pub(crate) async fn persist_running(store: &AppStore, spec: &SubtaskSpec) -> Result<(), ()> {
    let store = store.clone();
    let spec = spec.clone();
    tokio::task::spawn_blocking(move || {
        notifications::write(&store, (), |db| {
            super::repository::subtasks::insert(db, &spec).map_err(|_| ())?;
            Ok(((), Some(spec.parent_conversation_id)))
        })
    })
    .await
    .map_err(|_| ())?
}

/// Resolve the MCP catalog for a conversation through the shared child
/// boundary. A persisted child reconstructs its frozen allowlist from the
/// durable subtask row; ordinary root conversations retain the session-level
/// approved catalog. Before exposing either view, the installed runtime is
/// compared with the current app-owned configuration and revision-bound
/// approval receipts. A changed, missing, or unreadable configuration fails
/// closed and clears the stale runtime.
#[cfg(unix)]
pub(crate) async fn mcp_runtime_for_conversation(
    store: &AppStore,
    sessions: &Arc<AgentSessionService>,
    conversation_id: Uuid,
) -> Option<Arc<dyn super::mcp_runtime::McpToolProvider>> {
    let runtime = sessions.mcp_runtime().await?;
    let root = store.agent_configuration_directory().to_path_buf();
    let database = store.shared_agent_connection();
    let active_identities = tokio::task::spawn_blocking(move || {
        let configured = super::mcp_config::read_from_root(&root).ok().flatten()?;
        let db = database.lock().ok()?;
        let mut identities = configured
            .into_iter()
            .filter(|server| server.enabled)
            .filter_map(|server| {
                let approval = super::repository::mcp_approvals::read(&db, &server.server_id)
                    .ok()
                    .flatten()?;
                (approval.environment_id == server.environment_id
                    && approval.revision == server.revision)
                    .then_some((server.server_id, server.environment_id, server.revision))
            })
            .collect::<Vec<_>>();
        identities.sort();
        Some(identities)
    })
    .await
    .ok()
    .flatten();
    let Some(mut active_identities) = active_identities else {
        sessions.clear_mcp_runtime().await;
        return None;
    };
    if !runtime_identities_match(runtime.identities(), &mut active_identities) {
        sessions.clear_mcp_runtime().await;
        return None;
    }
    let db = store.shared_agent_connection();
    let child_scope = tokio::task::spawn_blocking(move || {
        db.lock().ok().and_then(|connection| {
            super::repository::subtasks::find_by_child(&connection, conversation_id)
                .ok()
                .flatten()
                .map(|record| record.approved_servers)
        })
    })
    .await
    .ok()
    .flatten();
    match child_scope {
        Some(approved_servers) => Some(runtime.scoped(approved_servers)),
        None => Some(runtime),
    }
}

#[cfg(test)]
mod mcp_identity_tests {
    use super::runtime_identities_match;

    #[test]
    fn identity_comparison_is_order_independent_and_revision_bound() {
        let installed = vec![
            ("zeta".into(), "env".into(), "r2".into()),
            ("alpha".into(), "env".into(), "r1".into()),
        ];
        let mut same = vec![
            ("alpha".into(), "env".into(), "r1".into()),
            ("zeta".into(), "env".into(), "r2".into()),
        ];
        assert!(runtime_identities_match(&installed, &mut same));
        let mut stale = vec![("alpha".into(), "env".into(), "r0".into())];
        assert!(!runtime_identities_match(&installed, &mut stale));
    }
}

pub(crate) async fn fail_and_archive(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    child_id: Uuid,
    request_id: Uuid,
) -> Result<(), super::child_archive::ArchiveFailure> {
    finish(store.clone(), child_id, TaskCompletion::Failed).await;
    super::child_archive::archive(store, catalog, child_id, request_id).await
}

/// Connect an ordinary host admission to the shared parent/child task owner.
/// The host runner remains the only model loop; this only adds lineage and
/// durable scheduling state around its existing task receipt.
pub(crate) fn track_started(
    store: &AppStore,
    sessions: &Arc<AgentSessionService>,
    parent_id: ConversationId,
    child_id: Uuid,
    owned: OwnedTurn,
) -> Result<(), ()> {
    if sessions
        .tasks()
        .link_child(parent_id, ConversationId::from_uuid(child_id))
        .is_err()
    {
        let _ = sessions
            .tasks()
            .cancel(ConversationId::from_uuid(child_id), owned.task.run_id());
        spawn_finish(store.clone(), child_id, owned);
        return Err(());
    }
    spawn_finish(store.clone(), child_id, owned);
    Ok(())
}

/// Reattach a recovered child to the parent lineage when that owner is still
/// live. After a process restart the durable parent relation remains valid but
/// there is no safe in-memory parent task to resurrect, so the child is
/// allowed to run independently rather than inventing a phantom owner.
pub(crate) fn track_started_after_restart(
    store: &AppStore,
    sessions: &Arc<AgentSessionService>,
    parent_id: ConversationId,
    child_id: Uuid,
    owned: OwnedTurn,
) -> Result<(), ()> {
    let linked = match sessions.tasks().has_active(parent_id) {
        Ok(false) => true,
        Ok(true) => sessions
            .tasks()
            .link_child(parent_id, ConversationId::from_uuid(child_id))
            .is_ok(),
        Err(_) => false,
    };
    if !linked {
        let _ = sessions
            .tasks()
            .cancel(ConversationId::from_uuid(child_id), owned.task.run_id());
        spawn_finish(store.clone(), child_id, owned);
        return Err(());
    }
    spawn_finish(store.clone(), child_id, owned);
    Ok(())
}

fn spawn_finish(store: AppStore, child_id: Uuid, owned: OwnedTurn) {
    tokio::spawn(async move {
        finish(store, child_id, owned.task.wait().await).await;
    });
}

async fn finish(store: AppStore, child_id: Uuid, completion: TaskCompletion) {
    let status = match completion {
        TaskCompletion::Completed => super::repository::subtasks::Status::Completed,
        TaskCompletion::Canceled => super::repository::subtasks::Status::Canceled,
        TaskCompletion::Failed => super::repository::subtasks::Status::Failed,
        TaskCompletion::RecoveryRequired => super::repository::subtasks::Status::RecoveryRequired,
    };
    let _ = tokio::task::spawn_blocking(move || {
        notifications::write(&store, (), |db| {
            let parent = notifications::parent(db, child_id).map_err(|_| ())?;
            super::repository::subtasks::finish(db, child_id, status).map_err(|_| ())?;
            Ok(((), parent))
        })
    })
    .await;
}

#[cfg(test)]
#[path = "child_runtime/tests.rs"]
mod tests;
