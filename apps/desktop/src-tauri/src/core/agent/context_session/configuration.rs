//! Native conversation configuration. Renderer storage is only a presentation cache.
use super::{Error, resource_grants, sources::Resource};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub revision: i64,
    pub state: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    pub conversation_id: Uuid,
    pub expected_revision: i64,
    pub state: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigurationEnvironment {
    schema_version: u8,
    operation_id: Uuid,
    conversation_id: Uuid,
    epoch: Uuid,
    sql_fence: String,
    configuration: Option<Configuration>,
}

#[tauri::command]
pub(crate) async fn agent_read_context_configuration_environment(
    store: tauri::State<'_, AppStore>,
    request: crate::data_interface::ContextConfigurationEnvironmentRequest,
) -> Result<ConfigurationEnvironment, crate::data_interface::ContextConfigurationFailure> {
    read_configuration_environment(&store, request).await
}

pub(crate) async fn read_configuration_environment(
    store: &AppStore,
    request: crate::data_interface::ContextConfigurationEnvironmentRequest,
) -> Result<ConfigurationEnvironment, crate::data_interface::ContextConfigurationFailure> {
    let operation_id = request.operation_id;
    let conversation_id = request.conversation_id;
    let database = store.shared_agent_connection();
    let (fence, configuration) =
        tokio::task::spawn_blocking(move || database.context_configuration_environment(&request))
            .await
            .map_err(|_| {
                crate::data_interface::ContextConfigurationFailure::Storage(
                    crate::data_interface::SqlImmediateFailure::RecoveryRequired(Box::new(
                        crate::data_interface::SqlImmediateDiagnostic::for_operation(
                            operation_id,
                            None,
                            "context-configuration-environment",
                            "configuration_worker_failed",
                        ),
                    )),
                )
            })??;
    Ok(ConfigurationEnvironment {
        schema_version: 1,
        operation_id,
        conversation_id,
        epoch: fence.epoch,
        sql_fence: fence.position.to_string(),
        configuration,
    })
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SaveStatus {
    Committed {
        proof: crate::agent::catalog_commands::CatalogMutationProofDto,
    },
    NoSqlReceipt {
        epoch: Uuid,
        sql_fence: String,
        closed: bool,
    },
    Unconfirmed {
        epoch: Uuid,
        sql_fence: String,
        blocked: bool,
        closed: bool,
        reason: &'static str,
    },
}

fn save_proof(
    conversation: Uuid,
    receipt: crate::data_interface::SqlImmediateReceipt,
) -> crate::agent::catalog_commands::CatalogMutationProofDto {
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
        "agent.context.configuration.save",
    )
}

fn seed_proof(
    conversation: Uuid,
    receipt: crate::data_interface::SqlImmediateReceipt,
) -> crate::agent::catalog_commands::CatalogMutationProofDto {
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
        "agent.context.configuration.seed",
    )
}

#[tauri::command]
pub(crate) async fn agent_seed_context_configuration_proven(
    store: tauri::State<'_, AppStore>,
    request: crate::data_interface::ContextConfigurationSeedIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::ContextConfigurationFailure,
> {
    seed_configuration_proven(&store, request).await
}

pub(crate) async fn seed_configuration_proven(
    store: &AppStore,
    request: crate::data_interface::ContextConfigurationSeedIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::ContextConfigurationFailure,
> {
    let operation = request.operation_id;
    let conversation = request.conversation_id;
    let database = store.shared_agent_connection();
    let writing_source = store.writing_resource_source();
    let reading_source = store.paper_reading_source();
    let analysis_source = store.paper_analysis_source();
    let receipt = tokio::task::spawn_blocking(move || {
        database.seed_context_configuration(
            &request,
            writing_source,
            reading_source,
            analysis_source,
        )
    })
    .await
    .map_err(|_| {
        crate::data_interface::ContextConfigurationFailure::Storage(
            crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
                crate::data_interface::SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "context-configuration-seed",
                    "configuration_worker_failed",
                ),
            )),
        )
    })??;
    Ok(seed_proof(conversation, receipt))
}

#[tauri::command]
pub(crate) async fn agent_read_context_configuration_seed_status(
    store: tauri::State<'_, AppStore>,
    request: crate::data_interface::ContextConfigurationSeedIntent,
) -> Result<SaveStatus, crate::data_interface::SqlImmediateFailure> {
    read_configuration_seed_status(&store, request).await
}

pub(crate) async fn read_configuration_seed_status(
    store: &AppStore,
    request: crate::data_interface::ContextConfigurationSeedIntent,
) -> Result<SaveStatus, crate::data_interface::SqlImmediateFailure> {
    let operation = request.operation_id;
    let conversation = request.conversation_id;
    let database = store.shared_agent_connection();
    let status =
        tokio::task::spawn_blocking(move || database.context_configuration_seed_status(&request))
            .await
            .map_err(|_| {
                crate::data_interface::SqlImmediateFailure::RecoveryRequired(Box::new(
                    crate::data_interface::SqlImmediateDiagnostic::for_operation(
                        operation,
                        None,
                        "context-configuration-seed-status",
                        "configuration_worker_failed",
                    ),
                ))
            })??;
    Ok(status_dto(conversation, status, seed_proof))
}

#[tauri::command]
pub(crate) async fn agent_save_context_configuration_proven(
    store: tauri::State<'_, AppStore>,
    request: crate::data_interface::ContextConfigurationIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::ContextConfigurationFailure,
> {
    save_configuration_proven(&store, request).await
}

pub(crate) async fn save_configuration_proven(
    store: &AppStore,
    request: crate::data_interface::ContextConfigurationIntent,
) -> Result<
    crate::agent::catalog_commands::CatalogMutationProofDto,
    crate::data_interface::ContextConfigurationFailure,
> {
    let operation = request.operation_id;
    let conversation = request.conversation_id;
    let database = store.shared_agent_connection();
    let writing_source = store.writing_resource_source();
    // Preserve the original SQL prefix. An admission failure is observed only
    // when validation actually requires the writing source.
    let needs_writing_scope = ["persistent", "temporary"]
        .into_iter()
        .filter_map(|key| request.state[key].as_array())
        .flatten()
        .any(|selection| selection["resource"]["kind"] == "manuscript");
    let native_scope = if let Some(binding) = store
        .native_writing_context()
        .filter(|_| needs_writing_scope)
    {
        Some(binding.scope_resources().await)
    } else {
        None
    };
    let receipt = tokio::task::spawn_blocking(move || match native_scope.as_ref() {
        Some(source) => database.save_context_configuration_with_scope(
            &request,
            Some(crate::agent::writing_resources::WritingScope::Native(
                source,
            )),
        ),
        None => database.save_context_configuration(&request, writing_source),
    })
    .await
    .map_err(|_| {
        crate::data_interface::ContextConfigurationFailure::Storage(
            crate::data_interface::SqlImmediateFailure::Unknown(Box::new(
                crate::data_interface::SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "context-configuration-save",
                    "configuration_worker_failed",
                ),
            )),
        )
    })??;
    Ok(save_proof(conversation, receipt))
}

#[tauri::command]
pub(crate) async fn agent_read_context_configuration_save_status(
    store: tauri::State<'_, AppStore>,
    request: crate::data_interface::ContextConfigurationIntent,
) -> Result<SaveStatus, crate::data_interface::SqlImmediateFailure> {
    read_configuration_save_status(&store, request).await
}

pub(crate) async fn read_configuration_save_status(
    store: &AppStore,
    request: crate::data_interface::ContextConfigurationIntent,
) -> Result<SaveStatus, crate::data_interface::SqlImmediateFailure> {
    let operation = request.operation_id;
    let conversation = request.conversation_id;
    let database = store.shared_agent_connection();
    let status =
        tokio::task::spawn_blocking(move || database.context_configuration_status(&request))
            .await
            .map_err(|_| {
                crate::data_interface::SqlImmediateFailure::RecoveryRequired(Box::new(
                    crate::data_interface::SqlImmediateDiagnostic::for_operation(
                        operation,
                        None,
                        "context-configuration-status",
                        "configuration_worker_failed",
                    ),
                ))
            })??;
    Ok(status_dto(conversation, status, save_proof))
}

fn status_dto(
    conversation: Uuid,
    status: crate::data_interface::SqlImmediateStatus,
    proof: fn(
        Uuid,
        crate::data_interface::SqlImmediateReceipt,
    ) -> crate::agent::catalog_commands::CatalogMutationProofDto,
) -> SaveStatus {
    match status {
        crate::data_interface::SqlImmediateStatus::Committed(receipt) => SaveStatus::Committed {
            proof: proof(conversation, receipt),
        },
        crate::data_interface::SqlImmediateStatus::NoSqlReceipt {
            epoch,
            sql_fence,
            closed,
        } => SaveStatus::NoSqlReceipt {
            epoch,
            sql_fence: sql_fence.position.to_string(),
            closed,
        },
        crate::data_interface::SqlImmediateStatus::Unconfirmed {
            epoch,
            sql_fence,
            blocked,
            closed,
            reason,
        } => SaveStatus::Unconfirmed {
            epoch,
            sql_fence: sql_fence.position.to_string(),
            blocked,
            closed,
            reason,
        },
    }
}
pub(crate) fn check(db: &Connection, conversation: Uuid, write: bool) -> Result<(), Error> {
    let exists: bool = db
        .query_row(
            "select exists(select 1 from agent_conversations where id=?1 and (?2=0 or archived=0))",
            params![conversation.to_string(), write],
            |row| row.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    if conversation.is_nil() || !exists {
        return Err(Error::ConversationUnavailable);
    }
    Ok(())
}

pub(crate) fn validate(
    db: &Connection,
    state: &Value,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), Error> {
    validate_with_scope(
        db,
        state,
        writing_source.map(crate::agent::writing_resources::WritingScope::Static),
    )
}

pub(crate) fn validate_with_scope(
    db: &Connection,
    state: &Value,
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<(), Error> {
    let object = state.as_object().ok_or(Error::InvalidInput)?;
    if state["schemaVersion"] != 1
        || object
            .keys()
            .any(|key| !matches!(key.as_str(), "schemaVersion" | "persistent" | "temporary"))
    {
        return Err(Error::InvalidInput);
    }
    let mut resources = Vec::new();
    let mut identities = std::collections::HashSet::new();
    for key in ["persistent", "temporary"] {
        for selection in state[key].as_array().ok_or(Error::InvalidInput)? {
            let object = selection.as_object().ok_or(Error::InvalidInput)?;
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "resource" | "label"))
                || selection["label"]
                    .as_str()
                    .is_none_or(|label| label.trim().is_empty() || label.len() > 4096)
            {
                return Err(Error::InvalidInput);
            }
            let resource: Resource = serde_json::from_value(selection["resource"].clone())
                .map_err(|_| Error::InvalidInput)?;
            let mut identity = serde_json::to_value(&resource).map_err(|_| Error::InvalidInput)?;
            if matches!(
                resource,
                Resource::StructuredObject { .. } | Resource::TemplateCatalog { .. }
            ) {
                identity
                    .as_object_mut()
                    .ok_or(Error::InvalidInput)?
                    .remove("access");
            }
            if !identities.insert(identity.to_string()) {
                return Err(Error::InvalidInput);
            }
            resources.push(resource);
        }
    }
    resource_grants::grants_with_scope(db, &resources, writing_source)?;
    Ok(())
}

pub(crate) fn read_in(db: &Connection, conversation: Uuid) -> Result<Option<Configuration>, Error> {
    check(db, conversation, false)?;
    db.query_row(
        "select revision,state_json from agent_context_configurations where conversation_id=?1",
        [conversation.to_string()],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(|_| Error::Unavailable)?
    .map(|(revision, state)| {
        Ok(Configuration {
            revision,
            state: serde_json::from_str(&state).map_err(|_| Error::Unavailable)?,
        })
    })
    .transpose()
}

/// Internal workflows consume the same saved selections as interactive turns.
/// An explicitly empty configuration is authoritative, never a missing seed.
pub(crate) fn resources_in(db: &Connection, conversation: Uuid) -> Result<Vec<Resource>, Error> {
    let saved = read_in(db, conversation)?.ok_or(Error::Unavailable)?;
    let mut resources = Vec::new();
    for key in ["persistent", "temporary"] {
        for selection in saved.state[key].as_array().ok_or(Error::InvalidInput)? {
            resources.push(
                serde_json::from_value(selection["resource"].clone())
                    .map_err(|_| Error::InvalidInput)?,
            );
        }
    }
    Ok(resources)
}

/// Startup migrates the renderer's old selections in its resource import transaction.
/// Native choices, including explicit emptiness, always win; archived history may
/// recover its configuration without gaining submission or configuration writes.
pub(crate) fn import_legacy_in(
    db: &Connection,
    conversation: Uuid,
    state: &Value,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), Error> {
    if conversation.is_nil() {
        return Err(Error::InvalidInput);
    }
    let exists: bool = db
        .query_row(
            "select exists(select 1 from agent_conversations where id=?1)",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    if !exists || read_in(db, conversation)?.is_some() {
        return Ok(());
    }
    if state.to_string().len() > 1_048_576 {
        return Err(Error::InvalidInput);
    }
    validate(db, state, writing_source)?;
    db.execute("insert into agent_context_configurations(conversation_id,revision,state_json,updated_at) values(?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(conversation_id) do nothing", params![conversation.to_string(),state.to_string()]).map_err(|_| Error::Unavailable)?;
    Ok(())
}

/// Workflow creation calls this inside its original transaction, before admission.
/// Existing user choices always win over a repeated or historical seed.
pub(crate) fn seed_in(
    db: &Connection,
    conversation: Uuid,
    resources: &[Resource],
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), Error> {
    check(db, conversation, true)?;
    let selections = resources
        .iter()
        .map(|resource| {
            Ok(json!({"resource":resource,"label":resource_grants::label(db, resource, writing_source)?}))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let state = json!({"schemaVersion":1,"persistent":selections,"temporary":[]});
    validate(db, &state, writing_source)?;
    db.execute("insert into agent_context_configurations(conversation_id,revision,state_json,updated_at) values(?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(conversation_id) do nothing", params![conversation.to_string(),state.to_string()]).map_err(|_| Error::Unavailable)?;
    Ok(())
}

/// Child creation confers authority only over the exact newly allocated object.
pub(crate) fn add_created_in(
    db: &Connection,
    conversation: Uuid,
    resource: &Resource,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), Error> {
    check(db, conversation, true)?;
    let Some(mut saved) = read_in(db, conversation)? else {
        return seed_in(
            db,
            conversation,
            std::slice::from_ref(resource),
            writing_source,
        );
    };
    let id = serde_json::to_value(resource).map_err(|_| Error::InvalidInput)?;
    if ["persistent", "temporary"].iter().any(|key| {
        saved.state[key].as_array().is_some_and(|selections| {
            selections.iter().any(|selection| {
                selection["resource"]["kind"] == id["kind"]
                    && selection["resource"]["resourceId"] == id["resourceId"]
            })
        })
    }) {
        return Ok(());
    }
    let persistent = saved.state["persistent"]
        .as_array_mut()
        .ok_or(Error::Unavailable)?;
    persistent.push(
        json!({"resource":resource,"label":resource_grants::label(db,resource, writing_source)?}),
    );
    validate(db, &saved.state, writing_source)?;
    let affected=db.execute("update agent_context_configurations set state_json=?2,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where conversation_id=?1 and revision=?3",params![conversation.to_string(),saved.state.to_string(),saved.revision]).map_err(|_|Error::Unavailable)?;
    if affected != 1 {
        return Err(Error::ConfirmationMismatch);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn agent_read_context_configuration(
    store: tauri::State<'_, AppStore>,
    conversation_id: Uuid,
    initialize_internal: Option<bool>,
) -> Result<Option<Configuration>, Error> {
    read_configuration(&store, conversation_id, initialize_internal).await
}

pub(crate) async fn read_configuration(
    store: &AppStore,
    conversation_id: Uuid,
    initialize_internal: Option<bool>,
) -> Result<Option<Configuration>, Error> {
    let db = store.connection().map_err(|_| Error::Unavailable)?;
    if read_in(&db, conversation_id)?.is_none() && initialize_internal.unwrap_or(true) {
        crate::ai::seed_existing_structured_context(
            store.paper_analysis_source(),
            store.paper_reading_source(),
            &db,
            conversation_id,
            store.writing_resource_source(),
        )
        .map_err(|_| Error::Unavailable)?;
    }
    read_in(&db, conversation_id)
}

#[tauri::command]
pub(crate) async fn agent_save_context_configuration(
    store: tauri::State<'_, AppStore>,
    request: SaveRequest,
) -> Result<Configuration, Error> {
    save_configuration(&store, request).await
}

pub(crate) async fn save_configuration(
    store: &AppStore,
    request: SaveRequest,
) -> Result<Configuration, Error> {
    if request.expected_revision < 0 || request.state.to_string().len() > 1_048_576 {
        return Err(Error::InvalidInput);
    }
    let mut db = store.connection().map_err(|_| Error::Unavailable)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| Error::Unavailable)?;
    check(&tx, request.conversation_id, true)?;
    validate(&tx, &request.state, store.writing_resource_source())?;
    let current = read_in(&tx, request.conversation_id)?;
    if current.as_ref().map_or(0, |saved| saved.revision) != request.expected_revision {
        return Err(Error::ConfirmationMismatch);
    }
    let revision = request
        .expected_revision
        .checked_add(1)
        .ok_or(Error::InvalidInput)?;
    tx.execute("insert into agent_context_configurations(conversation_id,revision,state_json,updated_at) values(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(conversation_id) do update set revision=excluded.revision,state_json=excluded.state_json,updated_at=excluded.updated_at",params![request.conversation_id.to_string(),revision,request.state.to_string()]).map_err(|_| Error::Unavailable)?;
    tx.commit().map_err(|_| Error::Unavailable)?;
    Ok(Configuration {
        revision,
        state: request.state,
    })
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
