//! Bounded child execution for page-note and reading-note Agent conversations.
//!
//! The frozen SubmissionInput is copied verbatim so a child cannot widen a
//! block selection by re-reading the mutable note. Execution then uses the
//! existing note admission and AgentExecution path.

use super::{
    child_creation, child_runtime,
    execution_profile::{AgentExecutionMode, execution_mode_for_model, execution_profile_for_host},
    model::AgentModelResolver,
    note_start::{self, StartRequest},
    reading_note_start,
    repository::{SqliteConversationCatalog, SqliteSubmissionStore},
    service::{AgentSessionService, SubmissionAdmission},
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationCatalog, ConversationId},
    submission::SubmissionStore,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub parent_conversation_id: Uuid,
    pub parent_turn_id: Uuid,
    pub page_id: Uuid,
    pub prompt: String,
    #[serde(default)]
    pub approved_servers: HashSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingRequest {
    pub parent_conversation_id: Uuid,
    pub parent_turn_id: Uuid,
    pub reading_note_id: Uuid,
    pub prompt: String,
    #[serde(default)]
    pub approved_servers: HashSet<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ChildResource {
    Page(Uuid),
    Reading(Uuid),
}

impl ChildResource {
    fn id(self) -> Uuid {
        match self {
            Self::Page(id) | Self::Reading(id) => id,
        }
    }

    fn binding(self) -> ConversationBinding {
        ConversationBinding {
            kind: match self {
                Self::Page(_) => BindingKind::Note,
                Self::Reading(_) => BindingKind::ReadingNote,
            },
            resource_id: self.id().to_string(),
        }
    }
}

struct SpawnChild {
    parent_conversation_id: Uuid,
    parent_turn_id: Uuid,
    resource: ChildResource,
    prompt: String,
    approved_servers: HashSet<String>,
}

struct StartChild {
    conversation_id: Uuid,
    request_id: Uuid,
    resource: ChildResource,
    context_snapshot_id: Uuid,
    model: research_agent_core::protocol::ModelIdentity,
    mode: AgentExecutionMode,
    model_calls: usize,
    tool_calls: usize,
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
#[serde(rename_all = "camelCase", tag = "code")]
pub enum Error {
    InvalidRequest,
    ParentNotRunning,
    ParentNotFound,
    StorageUnavailable,
    CatalogMutation(child_creation::CreationFailure),
    ChildSubmissionUnconfirmed(super::child_submission::SubmissionFailure),
    ArchiveMutation(super::child_archive::ArchiveFailure),
    ChildTrackingUnconfirmed(child_runtime::TrackingFailure),
    ExecutionUnavailable,
    RecoveryUnavailable,
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_spawn_note_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
) -> Result<Response, Error> {
    spawn_note_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        request,
    )
    .await
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_spawn_reading_note_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: ReadingRequest,
) -> Result<Response, Error> {
    spawn_reading_note_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        request,
    )
    .await
}

pub(crate) async fn spawn_note_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
) -> Result<Response, Error> {
    spawn_note_child_with_id(store, catalog, models, sessions, request, Uuid::new_v4()).await
}

pub(crate) async fn spawn_note_child_with_id(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
    child_id: Uuid,
) -> Result<Response, Error> {
    spawn_child(
        store,
        catalog,
        models,
        sessions,
        SpawnChild {
            parent_conversation_id: request.parent_conversation_id,
            parent_turn_id: request.parent_turn_id,
            resource: ChildResource::Page(request.page_id),
            prompt: request.prompt,
            approved_servers: request.approved_servers,
        },
        child_id,
    )
    .await
}

pub(crate) async fn spawn_reading_note_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: ReadingRequest,
) -> Result<Response, Error> {
    spawn_reading_note_child_with_id(store, catalog, models, sessions, request, Uuid::new_v4())
        .await
}

pub(crate) async fn spawn_reading_note_child_with_id(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: ReadingRequest,
    child_id: Uuid,
) -> Result<Response, Error> {
    spawn_child(
        store,
        catalog,
        models,
        sessions,
        SpawnChild {
            parent_conversation_id: request.parent_conversation_id,
            parent_turn_id: request.parent_turn_id,
            resource: ChildResource::Reading(request.reading_note_id),
            prompt: request.prompt,
            approved_servers: request.approved_servers,
        },
        child_id,
    )
    .await
}

async fn spawn_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    mut request: SpawnChild,
    child_id: Uuid,
) -> Result<Response, Error> {
    if request.resource.id().is_nil() || request.prompt.trim().is_empty() {
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
    request.approved_servers = approved_servers;
    let parent = ConversationId::from_uuid(request.parent_conversation_id);
    if !sessions
        .tasks()
        .has_active(parent)
        .map_err(|_| Error::ExecutionUnavailable)?
    {
        return Err(Error::ParentNotRunning);
    }
    catalog
        .read(parent, Some(request.resource.binding()))
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
    if parent_record.conversation_id != request.parent_conversation_id {
        return Err(Error::InvalidRequest);
    }
    let creation = child_creation::intent(
        child_id,
        request.resource.binding(),
        child_creation::Host::Note,
    );
    child_creation::create(&catalog, &creation)
        .await
        .map_err(Error::CatalogMutation)?;
    let request_id = research_agent_core::conversation::child_submission_request_id(child_id);
    let mut input = parent_record.input.clone();
    input.text = request.prompt;
    let child_submission = crate::data_interface::ChildSubmissionIntent {
        parent: request.parent_conversation_id,
        child: child_id,
        request: request_id,
        binding: creation.binding.clone(),
        input,
    };
    let submission_operation = child_submission.operation();
    let child_record = match catalog.prepare_child_submission(child_submission).await {
        Ok(record) => record,
        Err(error) => {
            if super::child_submission::uncertain(&error) {
                return Err(Error::ChildSubmissionUnconfirmed(
                    super::child_submission::SubmissionFailure {
                        child_id,
                        request_id,
                        operation_id: submission_operation,
                        reason: error.into(),
                    },
                ));
            }
            super::child_archive::archive_unsubmitted(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::StorageUnavailable);
        }
    };
    let mode = execution_mode_for_model(
        &child_record.input.model,
        child_record.input.reasoning_effort.as_deref(),
    );
    let profile = match execution_profile_for_host(
        &child_record.input.model,
        mode,
        super::model::AgentHost::Notes,
    )
    .and_then(|profile| profile.with_submission_compaction(&child_record.input))
    {
        Ok(profile) => profile,
        Err(_) => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
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
        child_context_snapshot_id: child_record.context_snapshot_id,
        parent_depth: bounds.depth,
        parent_model_calls: bounds.model_calls,
        parent_tool_calls: bounds.tool_calls,
        child_model_calls: bounds.model_calls.min(1),
        child_tool_calls: bounds.tool_calls.min(1),
        parent_approved_servers: request.approved_servers.clone(),
        child_approved_servers: request.approved_servers.clone(),
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
    let admission = match start_bounded_child(
        &store,
        &models,
        sessions.clone(),
        StartChild {
            conversation_id: child_id,
            request_id,
            resource: request.resource,
            context_snapshot_id: child_record.context_snapshot_id,
            model: child_record.input.model.clone(),
            mode,
            model_calls: child_spec.model_calls,
            tool_calls: child_spec.tool_calls,
        },
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
        context_snapshot_id: child_record.context_snapshot_id,
    };
    child_runtime::track_started(&store, &sessions, parent, child_id, owned)
        .map_err(|_| Error::ChildTrackingUnconfirmed(tracking))?;
    Ok(Response {
        schema_version: 1,
        child_conversation_id: child_id,
        request_id,
        turn_id: child_record.turn_id,
        context_snapshot_id: child_record.context_snapshot_id,
    })
}

async fn start_bounded_child(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartChild,
) -> Result<SubmissionAdmission, note_start::Error> {
    match request.resource {
        ChildResource::Page(page_id) => {
            note_start::start_bounded(
                store,
                models,
                sessions,
                StartRequest {
                    conversation_id: request.conversation_id,
                    request_id: request.request_id,
                    page_id,
                    context_snapshot_id: request.context_snapshot_id,
                    model: request.model,
                    mode: request.mode,
                },
                request.model_calls,
                request.tool_calls,
            )
            .await
        }
        ChildResource::Reading(reading_note_id) => {
            reading_note_start::start_bounded(
                store,
                models,
                sessions,
                reading_note_start::StartRequest {
                    conversation_id: request.conversation_id,
                    request_id: request.request_id,
                    reading_note_id,
                    context_snapshot_id: request.context_snapshot_id,
                    model: request.model,
                    mode: request.mode,
                },
                request.model_calls,
                request.tool_calls,
            )
            .await
        }
    }
}

/// Explicitly resume a note child after owner loss using its immutable
/// submission and the existing note admission path.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_resume_note_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    page_id: Uuid,
) -> Result<Response, Error> {
    resume_note_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        parent_conversation_id,
        child_conversation_id,
        page_id,
    )
    .await
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_resume_reading_note_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    reading_note_id: Uuid,
) -> Result<Response, Error> {
    resume_reading_note_child(
        store.inner().clone(),
        catalog.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        parent_conversation_id,
        child_conversation_id,
        reading_note_id,
    )
    .await
}

pub(crate) async fn resume_note_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    page_id: Uuid,
) -> Result<Response, Error> {
    resume_child(
        store,
        catalog,
        models,
        sessions,
        parent_conversation_id,
        child_conversation_id,
        ChildResource::Page(page_id),
    )
    .await
}

pub(crate) async fn resume_reading_note_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    reading_note_id: Uuid,
) -> Result<Response, Error> {
    resume_child(
        store,
        catalog,
        models,
        sessions,
        parent_conversation_id,
        child_conversation_id,
        ChildResource::Reading(reading_note_id),
    )
    .await
}

async fn resume_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    resource: ChildResource,
) -> Result<Response, Error> {
    if resource.id().is_nil() {
        return Err(Error::InvalidRequest);
    }
    let (task, record) = child_runtime::prepare_resume(
        &store,
        &catalog,
        parent_conversation_id,
        child_conversation_id,
        resource.binding(),
    )
    .await
    .map_err(|_| Error::RecoveryUnavailable)?;
    if task.child_context_snapshot_id != record.context_snapshot_id {
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
    let retry_record = match child_runtime::create_retry_submission(
        &store,
        child_conversation_id,
        &record,
        |_, _| Ok(()),
    )
    .await
    {
        Ok(record) => record,
        Err(_) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryUnavailable);
        }
    };
    let admission = start_bounded_child(
        &store,
        &models,
        sessions.clone(),
        StartChild {
            conversation_id: child_conversation_id,
            request_id: retry_record.request_id,
            resource,
            context_snapshot_id: retry_record.context_snapshot_id,
            model: retry_record.input.model.clone(),
            mode,
            model_calls: task.model_calls as usize,
            tool_calls: task.tool_calls as usize,
        },
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
