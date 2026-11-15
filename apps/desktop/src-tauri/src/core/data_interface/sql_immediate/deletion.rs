//! Atomic deletion metadata/proof. This module never performs file cleanup.
use super::*;
use crate::agent::repository::{
    DeletionSqlError, PreparedDeletion, delete_in, deletion_replay_sql, deletion_request_hash,
};
use research_agent_core::conversation::{
    CatalogError, ConversationDeletionError, ConversationDeletionRequest,
};
use sha2::{Digest, Sha256};
const KIND: &str = "agent.conversation.delete";
#[derive(Debug)]
pub(crate) enum DeletionFailure {
    Domain(ConversationDeletionError),
    Storage(SqlImmediateFailure),
    CleanupComplete(Box<SqlImmediateDiagnostic>),
}
impl From<SqlImmediateFailure> for DeletionFailure {
    fn from(value: SqlImmediateFailure) -> Self {
        Self::Storage(value)
    }
}
fn identity(request: &ConversationDeletionRequest) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    request.validate().map_err(SqlImmediateFailure::Domain)?;
    // Reuse the exact deployed full-intent hash without changing old tombstones.
    let bytes = format!("{KIND}:1:{}", deletion_request_hash(request));
    Ok(ReceiptIdentity {
        operation_id: request.request_id,
        kind: KIND,
        version: 1,
        fingerprint: Sha256::digest(bytes.as_bytes()).into(),
    })
}
fn raw<T>(
    result: Result<T, DeletionSqlError>,
    operation: Uuid,
    fence: ReadFence,
    phase: &'static str,
    control: &mut Option<ConversationDeletionError>,
) -> Result<T, SqlImmediateFailure> {
    result.map_err(|cause| match cause {
        DeletionSqlError::Sql(cause) => receipts::storage(operation, fence, phase, &cause),
        DeletionSqlError::Domain(ConversationDeletionError::Catalog(CatalogError::Corrupt)) => {
            SqlImmediateFailure::Corrupt(Box::new(super::diagnostic(
                operation,
                fence,
                phase,
                "delete_state_corrupt",
            )))
        }
        DeletionSqlError::Domain(cause) => {
            *control = Some(cause);
            SqlImmediateFailure::Domain(CatalogError::InvalidInput)
        }
    })
}
impl Database {
    pub(crate) fn complete_deleted_workspace_gc(
        &self,
        original: Uuid,
        conversation: Uuid,
        lease: Uuid,
    ) -> Result<(), SqlImmediateFailure> {
        let operation = Uuid::new_v5(
            &original,
            format!("workspace-gc:{conversation}:{lease}").as_bytes(),
        );
        let identity = ReceiptIdentity {
            operation_id: operation,
            kind: "agent.workspace.gc.complete",
            version: 1,
            fingerprint: Sha256::digest(format!("{original}:{conversation}:{lease}").as_bytes())
                .into(),
        };
        self.execute_sql(operation, |tx, fence| {
            receipts::check_state(tx, operation, fence, "workspace-gc")?;
            receipts::check_cross_kind(tx, operation, fence, "workspace-gc")?;
            if let Some(saved) = receipts::lookup(tx, &identity, fence, "workspace-gc")? { return Ok(saved); }
            let removed = tx.execute("delete from agent_deleted_workspace_gc where operation_id=?1 and conversation_id=?2 and lease_id=?3 and exists(select 1 from agent_conversation_deletions d where d.request_id=?1 and d.conversation_id=?2) and not exists(select 1 from agent_conversations c where c.id=?2)",
                rusqlite::params![original.to_string(), conversation.to_string(), lease.to_string()])
                .map_err(|cause| receipts::storage(operation, fence, "workspace-gc", &cause))?;
            if removed != 1 { return Err(SqlImmediateFailure::Domain(CatalogError::Conflict)); }
            receipts::insert(tx, &identity, fence)
        }, #[cfg(test)] super::Fault::None).map(|_| ())
    }
    /// Publish an entire prevalidated batch in one managed SQLite transaction.
    /// Workspaces are queued in the same commit and are never removed before it.
    pub(crate) fn delete_conversations_batch(
        &self,
        prepared: &[PreparedDeletion],
    ) -> Result<(), DeletionFailure> {
        let first = prepared
            .first()
            .ok_or(DeletionFailure::Domain(CatalogError::InvalidInput.into()))?;
        let operation = first.request().request_id;
        let mut control = None;
        let result = self.execute_sql(
            operation,
            |tx, fence| {
                let mut last = None;
                for item in prepared {
                    let request = item.request();
                    let identity = identity(request)?;
                    let operation = identity.operation_id;
                    receipts::check_state(tx, operation, fence, "delete-batch")?;
                    receipts::check_cross_kind(tx, operation, fence, "delete-batch")?;
                    if receipts::lookup(tx, &identity, fence, "delete-batch")?.is_some()
                        || raw(deletion_replay_sql(tx, request), operation, fence, "delete-batch", &mut control)?.is_some()
                    {
                        control = Some(CatalogError::Conflict.into());
                        return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput));
                    }
                    if request.force {
                        tx.execute(
                            "insert into agent_deleted_workspace_gc(operation_id,conversation_id,turn_id,manuscript_id,lease_id) select ?1,conversation_id,turn_id,manuscript_id,lease_id from agent_execution_workspaces where conversation_id=?2",
                            rusqlite::params![operation.to_string(), request.conversation_id.as_uuid().to_string()],
                        ).map_err(|cause| receipts::storage(operation, fence, "delete-batch-workspace-queue", &cause))?;
                    }
                    raw(delete_in(tx, item), operation, fence, "delete-batch", &mut control)?;
                    last = Some(receipts::insert(tx, &identity, fence)?);
                }
                last.ok_or(SqlImmediateFailure::Domain(CatalogError::InvalidInput))
            },
            #[cfg(test)]
            super::Fault::None,
        );
        match result {
            Ok(_) => Ok(()),
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) if control.is_some() => {
                Err(DeletionFailure::Domain(control.unwrap()))
            }
            Err(cause) => Err(cause.into()),
        }
    }
    /// No projection, receipt, workspace lookup or file access before rejection.
    pub(crate) fn check_deletion_epoch(
        &self,
        operation: Uuid,
        expected: Uuid,
    ) -> Result<(), SqlImmediateFailure> {
        self.check_catalog_epoch(operation, expected, "delete-epoch")
    }
    /// Validate original identity after draining and before any external file
    /// cleanup, while the catalog retains this exact admission/SQL guard.
    pub(crate) fn deletion_cleanup_needed(
        db: &mut crate::data_interface::DatabaseGuard<'_>,
        request: &ConversationDeletionRequest,
    ) -> Result<bool, DeletionFailure> {
        let identity = identity(request)?;
        let operation = identity.operation_id;
        let fence = db
            ._admission
            .as_ref()
            .ok_or_else(|| super::unmanaged(operation))?
            .fence();
        let mut control = None;
        let result = (|| {
            receipts::check_state(db, operation, fence, "delete-cleanup-prepare")?;
            receipts::check_cross_kind(db, operation, fence, "delete-cleanup-prepare")?;
            let managed = receipts::lookup(db, &identity, fence, "delete-cleanup-prepare")?;
            let saved = raw(
                deletion_replay_sql(db, request),
                operation,
                fence,
                "delete-cleanup-prepare",
                &mut control,
            )?
            .is_some();
            if managed.is_some() && !saved {
                return Err(SqlImmediateFailure::Corrupt(Box::new(super::diagnostic(
                    operation,
                    fence,
                    "delete-cleanup-prepare",
                    "delete_tombstone_missing",
                ))));
            }
            Ok(!saved)
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            db.require_recovery();
        }
        match result {
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) if control.is_some() => {
                Err(DeletionFailure::Domain(control.unwrap()))
            }
            other => other.map_err(Into::into),
        }
    }
    /// Read-only exact replay, including blocked/closed owners. Only an existing
    /// immutable tombstone can succeed here; absence never authorizes cleanup.
    pub(crate) fn deletion_replay(
        &self,
        request: &ConversationDeletionRequest,
    ) -> Result<bool, DeletionFailure> {
        let identity = identity(request)?;
        let operation = identity.operation_id;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| super::unmanaged(operation))?;
        let inspection = owner
            .inspection_guard()
            .map_err(|cause| super::admission_error(operation, cause, None))?;
        let state = inspection.snapshot();
        let cached = ReadFence {
            epoch: state.epoch,
            position: state.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::Unavailable(Box::new(super::diagnostic(
                operation,
                cached,
                "delete-replay-lock",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            return Err(
                SqlImmediateFailure::RecoveryRequired(Box::new(super::diagnostic(
                    operation,
                    cached,
                    "delete-replay",
                    "sql_transaction_already_open",
                )))
                .into(),
            );
        }
        let fence = receipts::status_fence(&db, operation, cached, state.accepted_position)?;
        receipts::check_cross_kind(&db, operation, fence, "delete-replay")?;
        let managed = receipts::lookup(&db, &identity, fence, "delete-replay")?;
        let mut control = None;
        let saved = match raw(
            deletion_replay_sql(&db, request),
            operation,
            fence,
            "delete-replay",
            &mut control,
        ) {
            Ok(value) => value.is_some(),
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) if control.is_some() => {
                return Err(DeletionFailure::Domain(control.unwrap()));
            }
            Err(cause) => return Err(cause.into()),
        };
        if managed.is_some() && !saved {
            return Err(SqlImmediateFailure::Corrupt(Box::new(super::diagnostic(
                operation,
                fence,
                "delete-replay",
                "delete_tombstone_missing",
            )))
            .into());
        }
        if !saved && (state.blocked || state.closed) {
            return Err(
                SqlImmediateFailure::RecoveryRequired(Box::new(super::diagnostic(
                    operation,
                    fence,
                    "delete-replay",
                    if state.blocked {
                        "owner_recovery_required"
                    } else {
                        "owner_closed"
                    },
                )))
                .into(),
            );
        }
        Ok(saved)
    }
    pub(crate) fn delete_conversation(
        &self,
        prepared: &PreparedDeletion,
    ) -> Result<(), DeletionFailure> {
        self.delete_conversation_owned(
            prepared,
            #[cfg(test)]
            super::Fault::None,
        )
    }
    fn delete_conversation_owned(
        &self,
        prepared: &PreparedDeletion,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<(), DeletionFailure> {
        let request = prepared.request();
        let identity = identity(request)?;
        let operation = identity.operation_id;
        let mut control = None;
        let mut legacy = false;
        let mut publication_fence = None;
        #[cfg(test)]
        let break_rollback = matches!(&fault, super::Fault::Rollback);
        let result = self.execute_sql(
            operation,
            |tx, fence| {
                publication_fence = Some(fence);
                let result = (|| {
                    receipts::check_state(tx, operation, fence, "delete")?;
                    receipts::check_cross_kind(tx, operation, fence, "delete")?;
                    let managed = receipts::lookup(tx, &identity, fence, "delete")?;
                    let saved = raw(
                        deletion_replay_sql(tx, request),
                        operation,
                        fence,
                        "delete-publication",
                        &mut control,
                    )?
                    .is_some();
                    if let Some(receipt) = managed {
                        if !saved {
                            return Err(SqlImmediateFailure::Corrupt(Box::new(super::diagnostic(
                                operation,
                                fence,
                                "delete-replay",
                                "delete_tombstone_missing",
                            ))));
                        }
                        return Ok(receipt);
                    }
                    if saved {
                        legacy = true;
                        return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput));
                    }
                    raw(
                        delete_in(tx, prepared),
                        operation,
                        fence,
                        "delete-publication",
                        &mut control,
                    )?;
                    receipts::insert(tx, &identity, fence)
                })();
                #[cfg(test)]
                if result.is_err() && (control.is_some() || legacy) && break_rollback {
                    tx.execute_batch("rollback").unwrap();
                }
                result
            },
            #[cfg(test)]
            fault,
        );
        match result {
            Ok(_) => Ok(()),
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) => match control {
                Some(cause) if prepared.cleanup_complete() => Err(
                    DeletionFailure::CleanupComplete(Box::new(super::diagnostic(
                        operation,
                        publication_fence.expect("transaction body was entered"),
                        "delete-publication",
                        match cause {
                            ConversationDeletionError::NotArchived => {
                                "delete_not_archived_after_workspace_cleanup"
                            }
                            ConversationDeletionError::Catalog(CatalogError::Conflict) => {
                                "delete_conflict_after_workspace_cleanup"
                            }
                            ConversationDeletionError::Catalog(CatalogError::NotFound) => {
                                "delete_not_found_after_workspace_cleanup"
                            }
                            _ => "delete_metadata_refused_after_workspace_cleanup",
                        },
                    ))),
                ),
                Some(cause) => Err(DeletionFailure::Domain(cause)),
                None if legacy => Ok(()),
                None => Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput).into()),
            },
            Err(cause) => Err(cause.into()),
        }
    }
    pub(crate) fn deletion_status(
        &self,
        request: &ConversationDeletionRequest,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&identity(request)?)
    }
}
#[cfg(test)]
#[path = "deletion/tests.rs"]
mod tests;
