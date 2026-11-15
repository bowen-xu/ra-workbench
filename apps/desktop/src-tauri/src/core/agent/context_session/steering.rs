//! Same-turn input for the conversation-owned context host.
use crate::{
    agent::{
        repository::steering::SqliteSteeringStore,
        steering_contract::{SteerError, SteerResponse},
    },
    storage::AppStore,
};
use research_agent_core::steering::{SteeringError, SteeringInput, SteeringScope};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HandoffScope {
    pub conversation_id: Uuid,
    pub expected_epoch: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandoffPage {
    schema_version: u8,
    conversation_id: Uuid,
    epoch: Uuid,
    items: Vec<HandoffItem>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffItem {
    intent: crate::data_interface::SteeringHandoffIntent,
    proof: crate::agent::catalog_commands::CatalogMutationProofDto,
    history: crate::data_interface::SteeringHandoffHistory,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandoffResolution {
    status: &'static str,
    proof: Option<crate::agent::catalog_commands::CatalogMutationProofDto>,
}

#[tauri::command]
pub(crate) async fn agent_read_steering_handoff_resolution(
    store: State<'_, AppStore>,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<HandoffResolution, crate::data_interface::SqlImmediateFailure> {
    read_handoff_resolution(&store, request).await
}

pub(crate) async fn read_handoff_resolution(
    store: &AppStore,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<HandoffResolution, crate::data_interface::SqlImmediateFailure> {
    let conversation = request.intent.conversation_id;
    let operation = request.intent.operation_id;
    let database = store.shared_agent_connection();
    let result =
        tokio::task::spawn_blocking(move || database.inspect_steering_handoff_resolution(&request))
            .await
            .map_err(|_| {
                crate::data_interface::SqlImmediateFailure::RecoveryRequired(Box::new(
                    crate::data_interface::SqlImmediateDiagnostic::for_operation(
                        operation,
                        None,
                        "steering-handoff-resolution",
                        "handoff_worker_failed",
                    ),
                ))
            })??;
    use crate::data_interface::SteeringHandoffResolution;
    let (status, receipt, family) = match result {
        SteeringHandoffResolution::Unconfirmed => ("unconfirmed", None, ""),
        SteeringHandoffResolution::Finished(receipt) => {
            ("finished", Some(receipt), "agent.steering.handoff.finish")
        }
        SteeringHandoffResolution::Released(receipt) => {
            ("released", Some(receipt), "agent.steering.handoff.release")
        }
        SteeringHandoffResolution::Canceled(receipt) => {
            ("canceled", Some(receipt), "agent.steering.handoff.cancel")
        }
    };
    let proof = receipt.map(|receipt| {
        crate::agent::catalog_commands::CatalogMutationProofDto::new(
            research_agent_core::conversation::CatalogMutationProof {
                operation_id: receipt.operation_id,
                conversation_id: research_agent_core::conversation::ConversationId::from_uuid(
                    conversation,
                ),
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position,
                replayed: receipt.replayed,
            },
            family,
        )
    });
    Ok(HandoffResolution { status, proof })
}

#[tauri::command]
pub(crate) async fn agent_read_steering_handoffs(
    store: State<'_, AppStore>,
    request: HandoffScope,
) -> Result<HandoffPage, crate::data_interface::SqlImmediateFailure> {
    read_handoffs(&store, request).await
}
pub(crate) async fn read_handoffs(
    store: &AppStore,
    request: HandoffScope,
) -> Result<HandoffPage, crate::data_interface::SqlImmediateFailure> {
    let database = store.shared_agent_connection();
    let conversation = request.conversation_id;
    let (epoch, rows) = tokio::task::spawn_blocking(move || {
        database.inspect_steering_handoffs(conversation, request.expected_epoch)
    })
    .await
    .map_err(|_| {
        crate::data_interface::SqlImmediateFailure::RecoveryRequired(Box::new(
            crate::data_interface::SqlImmediateDiagnostic::for_operation(
                conversation,
                None,
                "steering-handoff-status",
                "handoff_worker_failed",
            ),
        ))
    })??;
    let items = rows
        .into_iter()
        .map(
            |row: crate::data_interface::SteeringHandoffPrepared| HandoffItem {
                intent: row.intent,
                history: row.history,
                proof: crate::agent::catalog_commands::CatalogMutationProofDto::new(
                    research_agent_core::conversation::CatalogMutationProof {
                        operation_id: row.receipt.operation_id,
                        conversation_id:
                            research_agent_core::conversation::ConversationId::from_uuid(
                                conversation,
                            ),
                        epoch: row.receipt.epoch,
                        sql_fence: row.receipt.sql_fence.position,
                        replayed: row.receipt.replayed,
                    },
                    "agent.steering.handoff.prepare",
                ),
            },
        )
        .collect();
    Ok(HandoffPage {
        schema_version: 1,
        conversation_id: conversation,
        epoch,
        items,
    })
}

#[tauri::command]
pub(crate) async fn agent_prepare_steering_handoff(
    store: State<'_, AppStore>,
    request: crate::data_interface::SteeringHandoffIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    prepare_handoff(&store, request).await
}

pub(crate) async fn prepare_handoff(
    store: &AppStore,
    request: crate::data_interface::SteeringHandoffIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    let database = store.shared_agent_connection();
    let conversation = request.conversation_id;
    let operation = request.operation_id;
    let receipt = tokio::task::spawn_blocking(move || database.prepare_steering_handoff(&request))
        .await
        .map_err(|_| {
            crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
                crate::data_interface::SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "steering-handoff-prepare",
                    "handoff_worker_failed",
                ),
            ))
        })??;
    Ok(
        crate::agent::catalog_commands::CatalogMutationProofDto::new(
            research_agent_core::conversation::CatalogMutationProof {
                operation_id: receipt.operation_id,
                conversation_id: research_agent_core::conversation::ConversationId::from_uuid(
                    conversation,
                ),
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position,
                replayed: receipt.replayed,
            },
            "agent.steering.handoff.prepare",
        ),
    )
}

#[tauri::command]
pub(crate) async fn agent_finish_steering_handoff(
    store: State<'_, AppStore>,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    finish_handoff(&store, request).await
}

pub(crate) async fn finish_handoff(
    store: &AppStore,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    let database = store.shared_agent_connection();
    let conversation = request.intent.conversation_id;
    let operation = request.operation_id()?;
    let receipt = tokio::task::spawn_blocking(move || database.finish_steering_handoff(&request))
        .await
        .map_err(|_| {
            crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
                crate::data_interface::SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "steering-handoff-finish",
                    "handoff_worker_failed",
                ),
            ))
        })??;
    Ok(
        crate::agent::catalog_commands::CatalogMutationProofDto::new(
            research_agent_core::conversation::CatalogMutationProof {
                operation_id: receipt.operation_id,
                conversation_id: research_agent_core::conversation::ConversationId::from_uuid(
                    conversation,
                ),
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position,
                replayed: receipt.replayed,
            },
            "agent.steering.handoff.finish",
        ),
    )
}

#[tauri::command]
pub(crate) async fn agent_release_steering_handoff(
    store: State<'_, AppStore>,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    release_handoff(&store, request).await
}

pub(crate) async fn release_handoff(
    store: &AppStore,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    let database = store.shared_agent_connection();
    let conversation = request.intent.conversation_id;
    let operation = request.release_operation_id()?;
    let receipt = tokio::task::spawn_blocking(move || database.release_steering_handoff(&request))
        .await
        .map_err(|_| {
            crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
                crate::data_interface::SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "steering-handoff-release",
                    "handoff_worker_failed",
                ),
            ))
        })??;
    Ok(
        crate::agent::catalog_commands::CatalogMutationProofDto::new(
            research_agent_core::conversation::CatalogMutationProof {
                operation_id: receipt.operation_id,
                conversation_id: research_agent_core::conversation::ConversationId::from_uuid(
                    conversation,
                ),
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position,
                replayed: receipt.replayed,
            },
            "agent.steering.handoff.release",
        ),
    )
}

#[tauri::command]
pub(crate) async fn agent_cancel_unregistered_steering_handoff(
    store: State<'_, AppStore>,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    cancel_unregistered_handoff(&store, request).await
}

pub(crate) async fn cancel_unregistered_handoff(
    store: &AppStore,
    request: crate::data_interface::FinishSteeringHandoff,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::SqlImmediateFailure,
> {
    let database = store.shared_agent_connection();
    let conversation = request.intent.conversation_id;
    let operation = request.cancel_operation_id()?;
    let receipt = tokio::task::spawn_blocking(move || {
        database.cancel_unregistered_steering_handoff(&request)
    })
    .await
    .map_err(|_| {
        crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
            crate::data_interface::SqlImmediateDiagnostic::for_operation(
                operation,
                None,
                "steering-handoff-cancel",
                "handoff_worker_failed",
            ),
        ))
    })??;
    Ok(
        crate::agent::catalog_commands::CatalogMutationProofDto::new(
            research_agent_core::conversation::CatalogMutationProof {
                operation_id: receipt.operation_id,
                conversation_id: research_agent_core::conversation::ConversationId::from_uuid(
                    conversation,
                ),
                epoch: receipt.epoch,
                sql_fence: receipt.sql_fence.position,
                replayed: receipt.replayed,
            },
            "agent.steering.handoff.cancel",
        ),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub input_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}

#[tauri::command]
pub(crate) async fn agent_steer_context_turn(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<SteerResponse, SteerError> {
    steer(&store, request).await
}

pub(crate) async fn steer(store: &AppStore, request: Request) -> Result<SteerResponse, SteerError> {
    let scope = SteeringScope {
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
    };
    let resources = store.clone();
    SqliteSteeringStore::new(store.shared_agent_connection())
        .enqueue_with_resources(
            SteeringInput {
                scope,
                input_id: request.input_id,
                text: request.text,
                user_input: request.user_input,
                skill_instructions: vec![],
            },
            move |db| {
                super::check_conversation(db, scope.conversation_id).map_err(|error| match error {
                    super::Error::ConversationUnavailable => SteeringError::Closed,
                    _ => SteeringError::Unavailable,
                })
            },
            move |db, input| crate::agent::skills::steering::prepare(db, &resources, input),
        )
        .await
        .map_err(SteerError::from)?;
    Ok(SteerResponse {
        schema_version: 1,
        conversation_id: scope.conversation_id,
        turn_id: scope.turn_id,
        input_id: request.input_id,
    })
}
