//! Bounded child execution for paper-reading conversations.
//!
//! The child reuses the ordinary paper submission preparation and runner. This
//! module only creates a separate paper-bound conversation and wires its task
//! receipt into the shared child runtime.

use super::{
    child_creation, child_runtime,
    execution_profile::{execution_mode_for_model, execution_profile_for_host},
    model::AgentModelResolver,
    reading_start::{self, StartRequest},
    reading_submission::{self, PrepareRequest},
    repository::{SqliteConversationCatalog, SqliteSubmissionStore},
    service::{AgentSessionService, SubmissionAdmission},
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationCatalog, ConversationId},
    submission::SubmissionStore,
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub parent_conversation_id: Uuid,
    pub parent_turn_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
    pub expected_context_hash: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub approved_servers: HashSet<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub schema_version: u8,
    pub child_conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "reason")]
pub enum ChildPreparationReason {
    CreationReplayed,
    Preparation(reading_submission::Error),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code")]
pub enum Error {
    InvalidRequest,
    ParentNotRunning,
    ParentNotFound,
    StorageUnavailable,
    CatalogMutation(child_creation::CreationFailure),
    ChildPreparationUnconfirmed {
        child_id: Uuid,
        request_id: Uuid,
        reason: ChildPreparationReason,
    },
    ArchiveMutation(super::child_archive::ArchiveFailure),
    ChildTrackingUnconfirmed(child_runtime::TrackingFailure),
    ExecutionUnavailable,
    RecoveryUnavailable,
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_spawn_paper_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
) -> Result<Response, Error> {
    spawn_paper_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        request,
    )
    .await
}

pub(crate) async fn spawn_paper_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
) -> Result<Response, Error> {
    spawn_paper_child_with_id(store, catalog, models, sessions, request, Uuid::new_v4()).await
}

pub(crate) async fn spawn_paper_child_with_id(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
    child_id: Uuid,
) -> Result<Response, Error> {
    let parent = ConversationId::from_uuid(request.parent_conversation_id);
    if !sessions
        .tasks()
        .has_active(parent)
        .map_err(|_| Error::ExecutionUnavailable)?
    {
        return Err(Error::ParentNotRunning);
    }
    if request.paper_id.trim().is_empty()
        || request.paper_context_snapshot_id.trim().is_empty()
        || request
            .expected_context_hash
            .as_deref()
            .is_some_and(str::is_empty)
    {
        return Err(Error::InvalidRequest);
    }
    let approved_servers = child_runtime::mcp_runtime_for_conversation(
        &store,
        &sessions,
        request.parent_conversation_id,
    )
    .await
    .map(|runtime| runtime.approved_servers())
    .unwrap_or_default();
    catalog
        .read(
            parent,
            Some(ConversationBinding {
                kind: BindingKind::Paper,
                resource_id: request.paper_id.clone(),
            }),
        )
        .await
        .map_err(|error| match error {
            research_agent_core::conversation::CatalogError::NotFound => Error::ParentNotFound,
            _ => Error::StorageUnavailable,
        })?;
    let submissions = SqliteSubmissionStore::for_app(&store);
    let parent_record = submissions
        .get_by_turn(request.parent_conversation_id, request.parent_turn_id)
        .await
        .map_err(|_| Error::ParentNotFound)?;
    if parent_record.conversation_id != request.parent_conversation_id
        || parent_record.context_snapshot_id.is_nil()
    {
        return Err(Error::InvalidRequest);
    }
    let parent_scope = {
        let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
        db.query_row(
            "select paper_id,paper_context_snapshot_id,context_hash from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
            rusqlite::params![request.parent_conversation_id.to_string(), parent_record.request_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|_| Error::StorageUnavailable)?
        .ok_or(Error::ParentNotFound)?
    };
    if parent_scope.0 != request.paper_id
        || parent_scope.1 != request.paper_context_snapshot_id
        || request
            .expected_context_hash
            .as_deref()
            .is_some_and(|hash| hash != parent_scope.2)
    {
        return Err(Error::InvalidRequest);
    }
    let creation = child_creation::intent(
        child_id,
        ConversationBinding {
            kind: BindingKind::Paper,
            resource_id: request.paper_id.clone(),
        },
        child_creation::Host::Paper,
    );
    let created = child_creation::create(&catalog, &creation)
        .await
        .map_err(Error::CatalogMutation)?;
    let request_id = research_agent_core::conversation::child_submission_request_id(child_id);
    // The legacy paper preparation has no owned receipt proving whether this
    // child has already started. A replayed creation cannot authorize dispatch.
    if created.replayed {
        return Err(Error::ChildPreparationUnconfirmed {
            child_id,
            request_id,
            reason: ChildPreparationReason::CreationReplayed,
        });
    }
    let prepared = match reading_submission::prepare(
        &store,
        &models,
        PrepareRequest {
            user_input: vec![],
            conversation_id: child_id,
            request_id,
            paper_id: request.paper_id.clone(),
            paper_context_snapshot_id: request.paper_context_snapshot_id.clone(),
            expected_context_hash: parent_scope.2.clone(),
            retrieval: None,
            text: request.prompt,
            model: Some(super::model::ModelSelection {
                provider_id: parent_record.input.model.provider_id.clone(),
                model_id: parent_record.input.model.model_id.clone(),
                reasoning_effort: parent_record.input.reasoning_effort.clone(),
                service_tier: parent_record.input.service_tier.clone(),
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(reason) => {
            return Err(Error::ChildPreparationUnconfirmed {
                child_id,
                request_id,
                reason: ChildPreparationReason::Preparation(reason),
            });
        }
    };
    let mode = execution_mode_for_model(
        &prepared.input.model,
        prepared.input.reasoning_effort.as_deref(),
    );
    let profile = match execution_profile_for_host(
        &prepared.input.model,
        mode,
        super::model::AgentHost::Reading,
    )
    .and_then(|profile| profile.with_submission_compaction(&prepared.input))
    {
        Ok(profile) => profile,
        Err(_) => {
            super::child_archive::archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ExecutionUnavailable);
        }
    };
    let bounds = match child_runtime::resolve_parent_bounds(
        &store,
        request.parent_conversation_id,
        parent_record.context_snapshot_id,
        profile.limits,
    )
    .await
    {
        Ok(bounds) => bounds,
        Err(_) => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ExecutionUnavailable);
        }
    };
    let (parent_spec, child_spec) = child_runtime::bounded_specs(child_runtime::SpecInputs {
        parent_conversation_id: request.parent_conversation_id,
        child_conversation_id: child_id,
        parent_context_snapshot_id: parent_record.context_snapshot_id,
        child_context_snapshot_id: prepared.context_snapshot_id,
        parent_depth: bounds.depth,
        parent_model_calls: bounds.model_calls,
        parent_tool_calls: bounds.tool_calls,
        child_model_calls: bounds.model_calls.min(1),
        child_tool_calls: bounds.tool_calls.min(1),
        parent_approved_servers: approved_servers.clone(),
        child_approved_servers: approved_servers,
    });
    if research_agent_core::subtasks::validate_child(&parent_spec, &child_spec).is_err()
        || child_runtime::persist_running(&store, &child_spec)
            .await
            .is_err()
    {
        child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
            .await
            .map_err(Error::ArchiveMutation)?;
        return Err(Error::StorageUnavailable);
    }
    let admission = match reading_start::start_bounded(
        &store,
        &models,
        sessions.clone(),
        StartRequest {
            conversation_id: child_id,
            request_id,
            paper_id: request.paper_id,
            paper_context_snapshot_id: request.paper_context_snapshot_id,
            context_snapshot_id: prepared.context_snapshot_id,
            model: prepared.input.model.clone(),
            mode,
        },
        child_spec.model_calls,
        child_spec.tool_calls,
    )
    .await
    {
        Ok(admission) => admission,
        Err(_) => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ExecutionUnavailable);
        }
    };
    let owned = match admission {
        SubmissionAdmission::Started(owned) => owned,
        SubmissionAdmission::AlreadyStarted { .. } => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ExecutionUnavailable);
        }
    };
    let tracking = child_runtime::TrackingFailure {
        child_conversation_id: child_id,
        request_id,
        turn_id: owned.turn_id,
        context_snapshot_id: prepared.context_snapshot_id,
    };
    child_runtime::track_started(&store, &sessions, parent, child_id, owned)
        .map_err(|_| Error::ChildTrackingUnconfirmed(tracking))?;
    Ok(Response {
        schema_version: 1,
        child_conversation_id: child_id,
        request_id,
        turn_id: prepared.turn_id,
        context_snapshot_id: prepared.context_snapshot_id,
    })
}

/// Explicitly resume a paper child after owner loss. The paper scope and
/// frozen submission are read from durable storage, then the ordinary reading
/// admission path is reused.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_resume_paper_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    paper_id: String,
) -> Result<Response, Error> {
    resume_paper_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        parent_conversation_id,
        child_conversation_id,
        paper_id,
    )
    .await
}

pub(crate) async fn resume_paper_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    paper_id: String,
) -> Result<Response, Error> {
    if paper_id.trim().is_empty() {
        return Err(Error::InvalidRequest);
    }
    let (task, record) = child_runtime::prepare_resume(
        &store,
        &catalog,
        parent_conversation_id,
        child_conversation_id,
        ConversationBinding {
            kind: BindingKind::Paper,
            resource_id: paper_id.clone(),
        },
    )
    .await
    .map_err(|_| Error::RecoveryUnavailable)?;
    let scope = {
        let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
        db.query_row(
            "select paper_id,paper_context_snapshot_id,context_hash from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
            rusqlite::params![child_conversation_id.to_string(), record.request_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(|_| Error::StorageUnavailable)?.ok_or(Error::RecoveryUnavailable)?
    };
    if scope.0 != paper_id || task.child_context_snapshot_id != record.context_snapshot_id {
        return Err(Error::RecoveryUnavailable);
    }
    let mode = execution_mode_for_model(
        &record.input.model,
        record.input.reasoning_effort.as_deref(),
    );
    if !child_runtime::claim_resume(&store, child_conversation_id)
        .await
        .map_err(|_| Error::StorageUnavailable)?
    {
        return Err(Error::RecoveryUnavailable);
    }
    let source_request_id = record.request_id;
    let paper_scope = scope.clone();
    let retry_record = match child_runtime::create_retry_submission(
        &store,
        child_conversation_id,
        &record,
        move |tx, retry| {
            tx.execute(
                "insert into agent_paper_submission_scopes(
                    conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash
                 ) select conversation_id,?1,?2,paper_id,paper_context_snapshot_id,context_hash
                   from agent_paper_submission_scopes
                  where conversation_id=?3 and request_id=?4",
                rusqlite::params![
                    retry.request_id.to_string(),
                    retry.turn_id.to_string(),
                    child_conversation_id.to_string(),
                    source_request_id.to_string()
                ],
            )
            .map_err(|_| ())?;
            let exists: bool = tx
                .query_row(
                    "select exists(select 1 from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2 and paper_id=?3)",
                    rusqlite::params![
                        retry.conversation_id.to_string(),
                        retry.request_id.to_string(),
                        paper_scope.0
                    ],
                    |row| row.get(0),
                )
                .map_err(|_| ())?;
            exists.then_some(()).ok_or(())
        },
    )
    .await
    {
        Ok(record) => record,
        Err(_) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryUnavailable);
        }
    };
    let admission = reading_start::start_bounded(
        &store,
        &models,
        sessions.clone(),
        StartRequest {
            conversation_id: child_conversation_id,
            request_id: retry_record.request_id,
            paper_id: scope.0,
            paper_context_snapshot_id: scope.1,
            context_snapshot_id: retry_record.context_snapshot_id,
            model: retry_record.input.model.clone(),
            mode,
        },
        task.model_calls as usize,
        task.tool_calls as usize,
    )
    .await;
    let owned = match admission {
        Ok(SubmissionAdmission::Started(value)) => value,
        Ok(SubmissionAdmission::AlreadyStarted { .. }) | Err(_) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::ExecutionUnavailable);
        }
    };
    child_runtime::track_started_after_restart(
        &store,
        &sessions,
        ConversationId::from_uuid(parent_conversation_id),
        child_conversation_id,
        owned,
    )
    .map_err(|_| Error::ExecutionUnavailable)?;
    Ok(Response {
        schema_version: 1,
        child_conversation_id,
        request_id: retry_record.request_id,
        turn_id: retry_record.turn_id,
        context_snapshot_id: retry_record.context_snapshot_id,
    })
}

#[cfg(test)]
#[path = "reading_child_commands_tests.rs"]
mod tests;
