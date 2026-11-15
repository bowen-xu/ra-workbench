//! Application-owned consent operations. No renderer/model authorization inference.
use super::*;
use crate::agent::service::AgentSessionService;
use crate::ai::structured_resource_validation::Flow;
use std::sync::Arc;

impl WritingSubmissionService {
    /// Approve a freshly captured successor independently of the revoked lease.
    pub async fn approve_next_workspace(
        &self,
        prepared: PreparedWorkspace,
        generation: u32,
        predecessor: Uuid,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            consent_transaction(&store, |tx| {
                if !prepared.lease.changes()?.is_empty() {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                execution_workspaces::generation::approve_in(
                    tx,
                    &prepared.binding,
                    generation,
                    predecessor,
                    prepared.lease.manifest(),
                )?;
                Ok(())
            })?;
            Ok(prepared)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Approve the exact prepared manifest's execution scope. The application
    /// calls this automatically before a normal send; retaining the lease keeps
    /// another writer from changing the copy meanwhile.
    pub async fn approve_workspace(
        &self,
        prepared: PreparedWorkspace,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            consent_transaction(&store, |tx| {
                let binding = &prepared.binding;
                let (saved, manifest) =
                    execution_workspaces::read(tx, binding.conversation_id, binding.request_id)?
                        .ok_or(execution_workspaces::Error::Conflict)?;
                if &saved != binding || prepared.lease.manifest() != &manifest {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                prepared.lease.changes()?;
                execution_workspaces::approve_in(tx, binding)?;
                Ok(())
            })?;
            Ok(prepared)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// A native binding is required; this is not an unrestricted IPC cancellation
    /// command. Withdraw admission before acknowledging persistence, then await
    /// process cleanup. Dropping the waiter cannot skip the initial stop signal.
    pub async fn revoke_workspace(
        &self,
        sessions: Arc<AgentSessionService>,
        binding: Binding,
    ) -> Result<(), Error> {
        let store = self.store.clone();
        let stopping = sessions.clone();
        let conversation = binding.conversation_id;
        let turn = binding.turn_id;
        tokio::spawn(async move {
            let (result, stop_attempted) = tokio::task::spawn_blocking(move || {
                let mut stop_attempted = false;
                let result = consent_transaction(&store, |tx| {
                    execution_workspaces::revoke_binding_in(tx, &binding)?;
                    // Persist withdrawal even if process cancellation fails.
                    stop_attempted = true;
                    Ok(stopping.processes().stop_turn(conversation, turn))
                })
                .and_then(|stopped| stopped.map_err(|_| Error::RecoveryRequired));
                (result, stop_attempted)
            })
            .await
            .map_err(|_| Error::Storage)?;
            if stop_attempted {
                sessions
                    .processes()
                    .finish_turn(conversation, turn)
                    .await
                    .map_err(|_| Error::RecoveryRequired)?;
            }
            result
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(super) fn consent_transaction<T>(
    store: &AppStore,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    consent_with_source(store, |tx| operation(tx).map_err(Flow::original))
        .map_err(Flow::<Error, std::convert::Infallible>::into_static)
}

// A source pause is rolled back through the same confirmed settlement and
// recovery fence as every other consent failure. No intent can precede Need.
pub(super) fn consent_with_source<T, P>(
    store: &AppStore,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, Flow<Error, P>>,
) -> Result<T, Flow<Error, P>> {
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
    match operation(&tx) {
        Ok(value) => match tx.commit() {
            Ok(()) if db.is_autocommit() => Ok(value),
            _ => {
                if !db.is_autocommit() {
                    let _ = db.execute_batch("rollback");
                }
                db.require_recovery();
                Err(Flow::original(Error::RecoveryRequired))
            }
        },
        Err(error) => match tx.rollback() {
            Ok(()) if db.is_autocommit() => Err(error),
            _ => {
                db.require_recovery();
                Err(Flow::original(Error::RecoveryRequired))
            }
        },
    }
}
