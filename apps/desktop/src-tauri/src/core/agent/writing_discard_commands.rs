//! User-owned discard confirmation/recovery boundary. Never a model tool.
use super::writing_workspace_commands::{Confirmation, Error, Scope};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[path = "writing_discard_commands/actions.rs"]
pub mod actions;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscardConfirmation {
    operation_id: Uuid,
    binding: Confirmation,
    environment_id: String,
    inventory_hash: String,
}
impl DiscardConfirmation {
    fn native(&self) -> Result<super::repository::execution_workspaces::discard::Request, Error> {
        let request = super::repository::execution_workspaces::discard::Request {
            operation_id: self.operation_id,
            binding: self.binding.binding()?,
            environment_id: self.environment_id.clone(),
            inventory_hash: self.inventory_hash.clone(),
        };
        request.validate().map_err(|_| Error::InvalidInput)?;
        Ok(request)
    }
}
impl From<super::repository::execution_workspaces::discard::Request> for DiscardConfirmation {
    fn from(request: super::repository::execution_workspaces::discard::Request) -> Self {
        let b = request.binding;
        Self {
            operation_id: request.operation_id,
            binding: Confirmation {
                scope: Scope {
                    conversation_id: b.conversation_id,
                    request_id: b.request_id,
                    manuscript_id: b.manuscript_id,
                    context_snapshot_id: b.context_snapshot_id,
                },
                turn_id: b.turn_id,
                lease_id: b.lease_id,
                manifest_hash: b.manifest_hash,
            },
            environment_id: request.environment_id,
            inventory_hash: request.inventory_hash,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    schema_version: u8,
    #[serde(flatten)]
    confirmation: DiscardConfirmation,
    state: RecoveryState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum RecoveryState {
    Pending,
    Complete,
}

pub(crate) async fn lookup(store: AppStore, scope: Scope) -> Result<Option<Recovery>, Error> {
    scope.validate()?;
    tokio::task::spawn_blocking(move || {
        use super::repository::execution_workspaces::{self, discard};
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        let found = discard::recovery::find(
            &tx,
            scope.conversation_id,
            scope.request_id,
            scope.manuscript_id,
            scope.context_snapshot_id,
        )
        .map_err(|error| match error {
            execution_workspaces::Error::Conflict => Error::Conflict,
            execution_workspaces::Error::Denied => Error::Denied,
            execution_workspaces::Error::Storage => Error::Unavailable,
        })?;
        Ok(found.map(|(request, state)| Recovery {
            schema_version: 1,
            confirmation: request.into(),
            state: match state {
                discard::State::Pending => RecoveryState::Pending,
                discard::State::Complete => RecoveryState::Complete,
            },
        }))
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_find_writing_workspace_discard(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Option<Recovery>, Error> {
    lookup(store.inner().clone(), request).await
}
