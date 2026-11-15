//! Explicit whole-plan disk rollback. No automatic startup execution or editor fallback.
use super::*;
use crate::agent::repository::{
    change_sets,
    proposal_applications::{
        self as applications, ApplicationPhase, ApplicationScope, ApplicationStatus,
        ApplicationTarget, StoredApplication,
    },
};

impl WritingSubmissionService {
    /// Caller owns live editor coverage and manuscript coordination. Validate the
    /// entire target plan before any physical mutation; editor plans are rejected.
    pub async fn rollback_disk_proposal(
        &self,
        scope: ApplicationScope,
    ) -> Result<StoredApplication, Error> {
        self.rollback_plan(scope, false).await
    }

    /// Resume disk compensation only after every editor effect was confirmed
    /// restored (or never dispatched). Never infer restoration from buffer text.
    pub async fn rollback_proposal_remainder(
        &self,
        scope: ApplicationScope,
    ) -> Result<StoredApplication, Error> {
        self.rollback_plan(scope, true).await
    }

    async fn rollback_plan(
        &self,
        scope: ApplicationScope,
        allow_resolved_editors: bool,
    ) -> Result<StoredApplication, Error> {
        let store = self.store.clone();
        let (saved, changes) = tokio::task::spawn_blocking(move || -> Result<_, Error> {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let mut tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            let proposal = change_sets::read_file_operation_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
            )?;
            for file in &saved.files {
                if file.phase == ApplicationPhase::Conflict {
                    return Err(LeaseError::Conflict.into());
                }
                if file.target != ApplicationTarget::Disk {
                    if !allow_resolved_editors {
                        return Err(LeaseError::Conflict.into());
                    }
                    let receipt = applications::read_editor_restoration_in(&tx, scope, &file.path)?;
                    let resolved = match file.phase {
                        ApplicationPhase::Pending => receipt.is_none(),
                        ApplicationPhase::Restored => {
                            receipt.is_some_and(|r| r.result_revision.is_some())
                        }
                        _ => false,
                    };
                    if !resolved {
                        return Err(LeaseError::Conflict.into());
                    }
                }
            }
            if saved.status == ApplicationStatus::RolledBack {
                return Ok((saved, vec![]));
            }
            applications::require_recovery_in(&mut tx, scope)?;
            // No effects remain when every file was never dispatched or already
            // restored by an earlier attempt. Finish without fabricated receipts.
            if saved.files.iter().all(|file| {
                matches!(
                    file.phase,
                    ApplicationPhase::Pending | ApplicationPhase::Restored
                )
            }) {
                applications::finish_rollback_in(&mut tx, scope)?;
            }
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok((saved, proposal.changes))
        })
        .await
        .map_err(|_| Error::Storage)??;
        if saved.status == ApplicationStatus::RolledBack {
            return Ok(saved);
        }
        let mut latest = saved.clone();
        // Deterministic reverse path order; each primitive verifies current bytes
        // and the frozen index again. First uncertainty stops the whole attempt.
        for file in saved.files.iter().rev() {
            if matches!(
                file.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            ) {
                continue;
            }
            let change = changes
                .iter()
                .find(|change| change.path == file.path)
                .ok_or(LeaseError::Conflict)?;
            latest = match (change.before.is_some(), change.after.is_some()) {
                (false, true) => {
                    self.rollback_added_proposal_file(scope, file.path.clone())
                        .await?
                }
                (true, false) => {
                    self.rollback_removed_proposal_file(scope, file.path.clone())
                        .await?
                }
                (true, true) => {
                    self.rollback_replaced_proposal_file(scope, file.path.clone())
                        .await?
                }
                _ => return Err(LeaseError::Conflict.into()),
            };
        }
        Ok(latest)
    }
}
