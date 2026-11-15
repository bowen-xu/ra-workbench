use super::{Error, Receipt, restore};
use crate::{
    agent::{
        composed_tools::RejectingToolExecutor,
        execution::AgentExecution,
        execution_profile::{execution_mode_for_model, execution_profile_for_host},
        model::{AgentHost, AgentModelResolver, AgentModelSelection},
        repository::SqliteSubmissionStore,
        service::AgentSessionService,
    },
    storage::AppStore,
};
use research_agent_core::{conversation::ConversationId, tool_registry::OwnedToolRegistration};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub queue: Option<crate::agent::queue_contract::Confirmation>,
    #[serde(default)]
    pub resume_queue: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "code", content = "reason", rename_all = "camelCase")]
pub(crate) enum GuardedError {
    Context(Error),
    Storage(crate::data_interface::SqlImmediateFailure),
    RecoveryRequired,
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum StartStatus {
    Admitted {
        preview: Box<super::Preview>,
    },
    Started {
        result: crate::agent::submission_response::StartResponse,
    },
    StartedUnobserved {
        result: crate::agent::submission_response::StartResponse,
    },
    StartedSettled {
        result: crate::agent::submission_response::StartResponse,
    },
    LegacyStartedSettled {
        result: crate::agent::submission_response::StartResponse,
    },
    LegacyStartedObserved {
        result: crate::agent::submission_response::StartResponse,
    },
    Unconfirmed {
        detail: Box<crate::data_interface::SqlImmediateDiagnostic>,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartStatusResponse {
    schema_version: u8,
    intent: Request,
    expected_epoch: Uuid,
    status: StartStatus,
}

#[tauri::command]
pub(crate) async fn agent_read_context_start_status(
    store: State<'_, AppStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
    expected_epoch: Uuid,
) -> Result<StartStatusResponse, GuardedError> {
    read_status(store.inner(), sessions.inner(), request, expected_epoch).await
}

pub(crate) async fn read_status(
    store: &AppStore,
    sessions: &AgentSessionService,
    request: Request,
    expected_epoch: Uuid,
) -> Result<StartStatusResponse, GuardedError> {
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.context_snapshot_id.is_nil()
    {
        return Err(GuardedError::Context(Error::InvalidInput));
    }
    let database = store.shared_agent_connection();
    if !sessions.owns_data(&database) {
        return Err(GuardedError::RecoveryRequired);
    }
    let observed_request = request.clone();
    let observed = tokio::task::spawn_blocking(move || {
        database.check_catalog_epoch(
            observed_request.request_id,
            expected_epoch,
            "context-start-status-epoch",
        )?;
        Ok::<_, crate::data_interface::SqlImmediateFailure>(
            database.inspect_context_start(&observed_request),
        )
    })
    .await
    .map_err(|_| GuardedError::RecoveryRequired)?
    .map_err(GuardedError::Storage)?;
    let status = match observed {
        crate::data_interface::ContextStartInspection::Admitted(record) => {
            let mut preview = super::Preview::from(*record);
            preview.queue = request.queue;
            StartStatus::Admitted {
                preview: Box::new(preview),
            }
        }
        crate::data_interface::ContextStartInspection::Started {
            turn_id,
            outcome,
            owner_settled,
        } => {
            let result = crate::agent::submission_response::StartResponse {
                schema_version: 1,
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                turn_id,
                run_id: sessions
                    .tasks()
                    .find_turn_receipt(ConversationId::from_uuid(request.conversation_id), turn_id)
                    .map_err(|_| GuardedError::RecoveryRequired)?
                    .map(|receipt| receipt.run_id()),
                already_started: true,
                outcome,
            };
            if result.run_id.is_some() {
                StartStatus::Started { result }
            } else if owner_settled {
                StartStatus::StartedSettled { result }
            } else {
                StartStatus::StartedUnobserved { result }
            }
        }
        crate::data_interface::ContextStartInspection::LegacySettled { turn_id, outcome } => {
            StartStatus::LegacyStartedSettled {
                result: crate::agent::submission_response::StartResponse {
                    schema_version: 1,
                    conversation_id: request.conversation_id,
                    request_id: request.request_id,
                    turn_id,
                    run_id: None,
                    already_started: true,
                    outcome: Some(outcome),
                },
            }
        }
        crate::data_interface::ContextStartInspection::LegacyObservedCandidate {
            turn_id,
            outcome,
            fence,
        } => {
            match sessions
                .tasks()
                .find_turn_receipt(ConversationId::from_uuid(request.conversation_id), turn_id)
                .map_err(|_| GuardedError::RecoveryRequired)?
            {
                Some(receipt) => StartStatus::LegacyStartedObserved {
                    result: crate::agent::submission_response::StartResponse {
                        schema_version: 1,
                        conversation_id: request.conversation_id,
                        request_id: request.request_id,
                        turn_id,
                        run_id: Some(receipt.run_id()),
                        already_started: true,
                        outcome,
                    },
                },
                None => StartStatus::Unconfirmed {
                    detail: Box::new(
                        crate::data_interface::SqlImmediateDiagnostic::for_operation(
                            request.request_id,
                            Some(fence),
                            "context-start-status",
                            "context_start_admission_intent_unconfirmed",
                        ),
                    ),
                },
            }
        }
        crate::data_interface::ContextStartInspection::Unconfirmed(detail) => {
            StartStatus::Unconfirmed { detail }
        }
    };
    Ok(StartStatusResponse {
        schema_version: 1,
        intent: request,
        expected_epoch,
        status,
    })
}

#[tauri::command]
pub(crate) async fn agent_start_context_submission_guarded(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
    expected_epoch: Uuid,
) -> Result<crate::agent::submission_response::StartResponse, GuardedError> {
    start_guarded(
        &store,
        &models,
        sessions.inner().clone(),
        request,
        expected_epoch,
    )
    .await
}

pub(crate) async fn start_guarded(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
    expected_epoch: Uuid,
) -> Result<crate::agent::submission_response::StartResponse, GuardedError> {
    let database = store.shared_agent_connection();
    if !sessions.owns_data(&database) {
        return Err(GuardedError::RecoveryRequired);
    }
    let operation = request.request_id;
    tokio::task::spawn_blocking(move || {
        database.check_catalog_epoch(operation, expected_epoch, "context-start-epoch")
    })
    .await
    .map_err(|_| GuardedError::RecoveryRequired)?
    .map_err(GuardedError::Storage)?;
    start(store, models, sessions, request)
        .await
        .map_err(guarded_start_error)
}

fn guarded_start_error(error: Error) -> GuardedError {
    match error {
        Error::QueueAdmissionUnknown {
            request_id,
            phase,
            safe_cause,
            sqlite_code,
            sqlite_extended_code,
            ..
        } => {
            let mut diagnostic = crate::data_interface::SqlImmediateDiagnostic::for_operation(
                request_id, None, phase, safe_cause,
            );
            diagnostic.sqlite_code = sqlite_code;
            diagnostic.sqlite_extended_code = sqlite_extended_code;
            GuardedError::Storage(crate::data_interface::SqlImmediateFailure::Unknown(
                Box::new(diagnostic),
            ))
        }
        other => GuardedError::Context(other),
    }
}

#[tauri::command]
pub(crate) async fn agent_start_context_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
) -> Result<crate::agent::submission_response::StartResponse, Error> {
    start(&store, &models, sessions.inner().clone(), request).await
}

pub(crate) async fn start(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: Request,
) -> Result<crate::agent::submission_response::StartResponse, Error> {
    if !sessions.owns_data(&store.shared_agent_connection()) {
        return Err(Error::Unavailable);
    }
    let record = restore(
        store,
        Receipt {
            conversation_id: request.conversation_id,
            request_id: request.request_id,
        },
    )
    .await?;
    if record.context_snapshot_id != request.context_snapshot_id {
        return Err(Error::ConfirmationMismatch);
    }
    let profile = execution_profile_for_host(
        &record.input.model,
        execution_mode_for_model(
            &record.input.model,
            record.input.reasoning_effort.as_deref(),
        ),
        AgentHost::General,
    )
    .and_then(|profile| profile.with_submission_compaction(&record.input))
    .map_err(|_| Error::ModelUnavailable)?;
    let model = models
        .prepare(
            AgentHost::General,
            Some(AgentModelSelection {
                provider_id: record.input.model.provider_id.clone(),
                model_id: record.input.model.model_id.clone(),
                reasoning_effort: record.input.reasoning_effort.clone(),
                service_tier: record.input.service_tier.clone(),
            }),
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let transport = profile
        .transport_with_model_history(
            store,
            &record,
            model,
            Some(models.clone()),
            AgentHost::General,
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let mcp = crate::agent::child_runtime::mcp_runtime_for_conversation(
        store,
        &sessions,
        record.conversation_id,
    )
    .await
    .map(|runtime| {
        if transport.supports_namespace_tools() {
            runtime.namespaced_view()
        } else {
            runtime
        }
    });
    let host_tools = match mcp {
        Some(tools) => OwnedToolRegistration::new(tools.definitions(), tools),
        None => OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    };
    let host_tools =
        crate::agent::skills::application::registration(store.clone(), record.clone(), host_tools)
            .await
            .map_err(|_| Error::Unavailable)?;
    let host_tools = super::resource_tools::registration(store.clone(), record.clone(), host_tools);
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        compaction_strategy: Some(
            profile
                .compaction_strategy(transport.clone())
                .map_err(|_| Error::ModelUnavailable)?,
        ),
        model: record.input.model.clone(),
        transport,
        host_tools,
        system_instructions: crate::agent::host_instructions::GENERAL.into(),
        limits: profile.limits,
        max_output_tokens: profile.max_output_tokens,
        max_input_bytes: profile.max_input_bytes,
    });
    admit_queue(store, record.clone(), request.queue, request.resume_queue).await?;
    let admitted = execution
        .start_with_steering(
            sessions.clone(),
            Arc::new(SqliteSubmissionStore::for_app(store)),
            ConversationId::from_uuid(request.conversation_id),
            request.request_id,
            Some(Arc::new(
                crate::agent::repository::steering::SqliteSteeringStore::new(
                    store.shared_agent_connection(),
                ),
            )),
        )
        .await
        .map_err(|_| Error::Unavailable)?;
    let response = crate::agent::submission_response::StartResponse::from_admission(
        request.conversation_id,
        request.request_id,
        admitted,
    )
    .with_retained_owner(sessions.tasks())
    .map_err(|_| Error::Unavailable)?;
    if response.already_started && response.run_id.is_none() {
        let database = store.shared_agent_connection();
        let inspected_request = request.clone();
        let settled =
            tokio::task::spawn_blocking(move || database.inspect_context_start(&inspected_request))
                .await
                .map_err(|_| Error::Unavailable)?;
        if !matches!(settled, crate::data_interface::ContextStartInspection::Started {
            turn_id, owner_settled:true, ..
        } if turn_id == response.turn_id)
        {
            return Err(Error::TaskObservationUnavailable);
        }
    }
    Ok(response)
}

async fn admit_queue(
    store: &AppStore,
    record: research_agent_core::submission::SubmissionRecord,
    confirmation: Option<crate::agent::queue_contract::Confirmation>,
    resume: bool,
) -> Result<(), Error> {
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        super::queue::admit_managed(&mut db, &record, confirmation, resume)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
