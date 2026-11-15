//! Native child execution for writing conversations.
//! Reuses the ordinary writing submission path instead of introducing a second
//! model runner; only the conversation identity and frozen prompt differ.

use super::{
    child_creation, child_runtime,
    execution_profile::{execution_mode_for_model, execution_profile_for_host},
    model::{AgentHost, AgentModelResolver},
    repository::{SqliteConversationCatalog, SqliteSubmissionStore},
    service::AgentSessionService,
    writing_submission::{
        WritingExecutionConfirmation, WritingSubmissionReceipt, WritingSubmissionService,
    },
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationCatalog, ConversationId},
    submission::SubmissionStore,
    subtasks::SpawnRequest,
};
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
    pub manuscript_id: Uuid,
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
    RecoveryRestoreUnavailable,
    RecoveryAdmissionUnavailable,
    RecoveryTrackingUnavailable,
    RecoveryParentBindingUnavailable,
    RecoveryChildBindingUnavailable,
    RecoveryTaskUnavailable,
    RecoverySubmissionUnavailable,
    RecoveryContextUnavailable,
    RecoveryTurnUnavailable,
    RecoveryValidationUnavailable,
    RecoveryModelUnavailable,
    ChildPreparationUnavailable,
    ChildAdmissionUnavailable,
    ChildTrackingUnavailable,
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_spawn_writing_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
) -> Result<Response, Error> {
    spawn_writing_child(
        store.inner().clone(),
        catalog.inner().clone(),
        service.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        request,
    )
    .await
}

/// Shared child admission path used by the Tauri command and native tool
/// adapters. This keeps child creation, budget inheritance and cleanup in one
/// implementation instead of growing a second model loop.
pub(crate) async fn spawn_writing_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    service: WritingSubmissionService,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
) -> Result<Response, Error> {
    spawn_writing_child_with_id(
        store,
        catalog,
        service,
        models,
        sessions,
        request,
        Uuid::new_v4(),
    )
    .await
}

pub(crate) async fn spawn_writing_child_with_id(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    service: WritingSubmissionService,
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
    catalog
        .read(
            parent,
            Some(ConversationBinding {
                kind: BindingKind::Manuscript,
                resource_id: request.manuscript_id.to_string(),
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
    if parent_record.conversation_id != request.parent_conversation_id {
        return Err(Error::InvalidRequest);
    }
    // The child request must carry the parent's frozen context identity. A
    // fresh UUID here would pass shape validation while silently severing the
    // provenance link that bounds child disclosure.
    let approved_servers = child_runtime::mcp_runtime_for_conversation(
        &store,
        &sessions,
        request.parent_conversation_id,
    )
    .await
    .map(|runtime| runtime.approved_servers())
    .unwrap_or_default();
    let spawn = SpawnRequest {
        prompt: request.prompt,
        context_snapshot_id: parent_record.context_snapshot_id,
        model_calls: 1,
        tool_calls: 1,
        approved_servers,
    };
    spawn.validate().map_err(|_| Error::InvalidRequest)?;

    let creation = child_creation::intent(
        child_id,
        ConversationBinding {
            kind: BindingKind::Manuscript,
            resource_id: request.manuscript_id.to_string(),
        },
        child_creation::Host::Writing,
    );
    child_creation::create(&catalog, &creation)
        .await
        .map_err(Error::CatalogMutation)?;
    let request_id = research_agent_core::conversation::child_submission_request_id(child_id);
    let mut input = parent_record.input.clone();
    input.text = spawn.prompt;
    let child_submission = crate::data_interface::ChildSubmissionIntent {
        parent: request.parent_conversation_id,
        child: child_id,
        request: request_id,
        binding: creation.binding.clone(),
        input,
    };
    let submission_operation = child_submission.operation();
    let submitted = catalog.prepare_child_submission(child_submission).await;
    let child_record = match submitted {
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
    let mut profile =
        match execution_profile_for_host(&child_record.input.model, mode, AgentHost::Writing)
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
    let parent_model_calls = bounds.model_calls;
    let parent_tool_calls = bounds.tool_calls;
    let child_model_calls = spawn.model_calls.min(parent_model_calls);
    let child_tool_calls = spawn.tool_calls.min(parent_tool_calls);
    profile.limits = profile
        .limits
        .with_call_budget(child_model_calls, child_tool_calls);
    let (parent_spec, child_spec) = child_runtime::bounded_specs(child_runtime::SpecInputs {
        parent_conversation_id: request.parent_conversation_id,
        child_conversation_id: child_id,
        parent_context_snapshot_id: parent_record.context_snapshot_id,
        child_context_snapshot_id: child_record.context_snapshot_id,
        parent_depth: bounds.depth,
        parent_model_calls,
        parent_tool_calls,
        child_model_calls,
        child_tool_calls,
        parent_approved_servers: spawn.approved_servers.clone(),
        child_approved_servers: spawn.approved_servers.clone(),
    });
    if research_agent_core::subtasks::validate_child(&parent_spec, &child_spec).is_err() {
        child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
            .await
            .map_err(Error::ArchiveMutation)?;
        return Err(Error::ExecutionUnavailable);
    }
    if child_runtime::persist_running(&store, &child_spec)
        .await
        .is_err()
    {
        child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
            .await
            .map_err(Error::ArchiveMutation)?;
        return Err(Error::StorageUnavailable);
    }
    let prepared = match service
        .restore(
            &models,
            WritingSubmissionReceipt {
                conversation_id: child_id,
                request_id,
                manuscript_id: request.manuscript_id,
            },
        )
        .await
    {
        Ok(prepared) => prepared,
        Err(_) => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ChildPreparationUnavailable);
        }
    };
    let confirmation = WritingExecutionConfirmation {
        receipt: WritingSubmissionReceipt {
            conversation_id: child_id,
            request_id,
            manuscript_id: request.manuscript_id,
        },
        context_snapshot_id: child_record.context_snapshot_id,
        model: child_record.input.model.clone(),
    };
    let admission = match service
        .start_confirmed(sessions.clone(), prepared, confirmation, profile)
        .await
    {
        Ok(admission) => admission,
        Err(_) => {
            child_runtime::fail_and_archive(&store, &catalog, child_id, request_id)
                .await
                .map_err(Error::ArchiveMutation)?;
            return Err(Error::ChildAdmissionUnavailable);
        }
    };
    let owned = match admission {
        super::service::SubmissionAdmission::Started(owned) => owned,
        super::service::SubmissionAdmission::AlreadyStarted { .. } => {
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

/// Explicitly resume a writing child whose owner was lost during a process
/// restart. The frozen submission is read from durable storage; current editor
/// contents and renderer supplied model/context overrides are never consulted.
#[tauri::command]
#[allow(private_interfaces)]
#[allow(clippy::too_many_arguments)]
pub async fn agent_resume_writing_child(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    manuscript_id: Uuid,
) -> Result<Response, Error> {
    resume_writing_child(
        store.inner().clone(),
        catalog.inner().clone(),
        service.inner().clone(),
        models.inner().clone(),
        sessions.inner().clone(),
        parent_conversation_id,
        child_conversation_id,
        manuscript_id,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resume_writing_child(
    store: AppStore,
    catalog: SqliteConversationCatalog,
    service: WritingSubmissionService,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
    manuscript_id: Uuid,
) -> Result<Response, Error> {
    let (child_record, record) = child_runtime::prepare_resume(
        &store,
        &catalog,
        parent_conversation_id,
        child_conversation_id,
        ConversationBinding {
            kind: BindingKind::Manuscript,
            resource_id: manuscript_id.to_string(),
        },
    )
    .await
    .map_err(|error| match error {
        child_runtime::ResumePreparationError::ParentBinding => {
            Error::RecoveryParentBindingUnavailable
        }
        child_runtime::ResumePreparationError::ChildBinding => {
            Error::RecoveryChildBindingUnavailable
        }
        child_runtime::ResumePreparationError::Task => Error::RecoveryTaskUnavailable,
        child_runtime::ResumePreparationError::Submission => Error::RecoverySubmissionUnavailable,
        child_runtime::ResumePreparationError::ContextSnapshot => Error::RecoveryContextUnavailable,
        child_runtime::ResumePreparationError::InvalidIdentity => Error::RecoveryUnavailable,
    })?;
    let mode = execution_mode_for_model(
        &record.input.model,
        record.input.reasoning_effort.as_deref(),
    );
    let mut profile = execution_profile_for_host(&record.input.model, mode, AgentHost::Writing)
        .and_then(|profile| profile.with_submission_compaction(&record.input))
        .map_err(|_| Error::ExecutionUnavailable)?;
    profile.limits = profile.limits.with_call_budget(
        child_record.model_calls as usize,
        child_record.tool_calls as usize,
    );
    if profile.limits.model_calls == Some(0) || profile.limits.tool_calls == Some(0) {
        return Err(Error::RecoveryUnavailable);
    }
    let claimed = child_runtime::claim_resume(&store, child_conversation_id)
        .await
        .map_err(|_| Error::StorageUnavailable)?;
    if !claimed {
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
            return Err(Error::RecoverySubmissionUnavailable);
        }
    };
    let receipt = WritingSubmissionReceipt {
        conversation_id: child_conversation_id,
        request_id: retry_record.request_id,
        manuscript_id,
    };
    let prepared = match service.restore(&models, receipt).await {
        Ok(value) => value,
        Err(_) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryRestoreUnavailable);
        }
    };
    let confirmation = WritingExecutionConfirmation {
        receipt,
        context_snapshot_id: retry_record.context_snapshot_id,
        model: retry_record.input.model.clone(),
    };
    let admission = match service
        .start_confirmed(sessions.clone(), prepared, confirmation, profile)
        .await
    {
        Ok(super::service::SubmissionAdmission::Started(owned)) => owned,
        Ok(super::service::SubmissionAdmission::AlreadyStarted { .. }) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryAdmissionUnavailable);
        }
        Err(super::writing_submission::WritingStartError::Mismatch) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryValidationUnavailable);
        }
        Err(super::writing_submission::WritingStartError::Preparation(_)) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryRestoreUnavailable);
        }
        Err(super::writing_submission::WritingStartError::Model(_)) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryModelUnavailable);
        }
        Err(super::writing_submission::WritingStartError::Run(
            super::service::SubmissionRunError::Session(super::service::AgentServiceError::NotOpen),
        )) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryTurnUnavailable);
        }
        Err(_) => {
            child_runtime::release_resume(&store, child_conversation_id).await;
            return Err(Error::RecoveryAdmissionUnavailable);
        }
    };
    child_runtime::track_started_after_restart(
        &store,
        &sessions,
        ConversationId::from_uuid(parent_conversation_id),
        child_conversation_id,
        admission,
    )
    .map_err(|_| Error::RecoveryTrackingUnavailable)?;
    Ok(Response {
        schema_version: 1,
        child_conversation_id,
        request_id: retry_record.request_id,
        turn_id: retry_record.turn_id,
        context_snapshot_id: retry_record.context_snapshot_id,
    })
}
