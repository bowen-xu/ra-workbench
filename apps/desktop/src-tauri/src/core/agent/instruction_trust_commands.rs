//! User settings only. Never registered as model tools; no caller-supplied paths.
use super::{instructions::inspect_instruction_root, repository::instruction_trust};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub manuscript_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRequest {
    pub manuscript_id: Uuid,
    pub expected_revision: u64,
    pub granted: bool,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustResponse {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub revision: u64,
    pub granted: bool,
    pub root_matches: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustCommandError {
    InvalidInput,
    Conflict,
    Unavailable,
}
fn error(value: instruction_trust::TrustError) -> TrustCommandError {
    match value {
        instruction_trust::TrustError::Conflict => TrustCommandError::Conflict,
        _ => TrustCommandError::Unavailable,
    }
}
fn root(store: &AppStore, id: Uuid) -> Result<std::path::PathBuf, TrustCommandError> {
    Ok(store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| TrustCommandError::Unavailable)?
        .join(id.to_string())
        .join("source"))
}
fn read(store: &AppStore, id: Uuid) -> Result<TrustResponse, TrustCommandError> {
    if id.is_nil() {
        return Err(TrustCommandError::InvalidInput);
    }
    let db = store
        .connection()
        .map_err(|_| TrustCommandError::Unavailable)?;
    let state = instruction_trust::read(&db, id).map_err(error)?;
    let root_matches = if state.as_ref().is_some_and(|s| s.granted) {
        root(store, id)
            .ok()
            .and_then(|path| {
                inspect_instruction_root(&path, |identity| {
                    instruction_trust::is_trusted(&db, id, identity)
                })
                .ok()
            })
            .transpose()
            .map_err(error)?
            .unwrap_or(false)
    } else {
        false
    };
    Ok(TrustResponse {
        schema_version: 1,
        manuscript_id: id,
        revision: state.as_ref().map_or(0, |s| s.revision),
        granted: state.is_some_and(|s| s.granted),
        root_matches,
    })
}
fn update(store: &AppStore, request: UpdateRequest) -> Result<TrustResponse, TrustCommandError> {
    if request.manuscript_id.is_nil() || request.expected_revision >= 9_007_199_254_740_991 {
        return Err(TrustCommandError::InvalidInput);
    }
    let mut db = store
        .connection()
        .map_err(|_| TrustCommandError::Unavailable)?;
    let revision = if request.granted {
        let path = root(store, request.manuscript_id)?;
        inspect_instruction_root(&path, |identity| {
            instruction_trust::update(
                &mut db,
                request.manuscript_id,
                request.expected_revision,
                identity,
                true,
            )
        })
        .map_err(|_| TrustCommandError::Unavailable)?
        .map_err(error)?
        .revision
    } else {
        instruction_trust::revoke(&mut db, request.manuscript_id, request.expected_revision)
            .map_err(error)?
    };
    Ok(TrustResponse {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        revision,
        granted: request.granted,
        root_matches: request.granted,
    })
}
#[tauri::command]
pub(crate) async fn agent_read_manuscript_instruction_trust(
    store: State<'_, AppStore>,
    request: ReadRequest,
) -> Result<TrustResponse, TrustCommandError> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || read(&store, request.manuscript_id))
        .await
        .map_err(|_| TrustCommandError::Unavailable)?
}
#[tauri::command]
pub(crate) async fn agent_update_manuscript_instruction_trust(
    store: State<'_, AppStore>,
    request: UpdateRequest,
) -> Result<TrustResponse, TrustCommandError> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || update(&store, request))
        .await
        .map_err(|_| TrustCommandError::Unavailable)?
}
#[cfg(test)]
#[path = "instruction_trust_commands/tests.rs"]
mod tests;
