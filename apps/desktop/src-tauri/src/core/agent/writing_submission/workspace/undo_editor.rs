//! One editor transaction for a prepared inverse plan, with durable receipts.
use super::*;
use crate::agent::{
    repository::{
        proposal_applications::{
            ApplicationPhase, ApplicationScope, ApplicationTarget, DispatchAdmission, FileReceipt,
        },
        proposal_undos::{self, StoredUndo},
    },
    workspace::ProposalObjects,
};

pub enum UndoEditorAdmission {
    Dispatch(EditorDispatch),
    Existing(StoredUndo),
}

impl WritingSubmissionService {
    /// The host holds the same complete manuscript lease from start through ACK.
    pub async fn prepare_editor_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        current: EditorObservation,
    ) -> Result<UndoEditorAdmission, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_read().map_err(|_| Error::Storage)?;
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let saved =
                proposal_undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            let file = saved
                .application
                .files
                .iter()
                .find(|f| f.path == current.path)
                .ok_or(LeaseError::Conflict)?;
            let ApplicationTarget::Editor { expected, .. } = &file.target else {
                return Err(LeaseError::Conflict.into());
            };
            if file.phase != ApplicationPhase::Pending {
                return Ok(UndoEditorAdmission::Existing(saved));
            }
            let change = saved
                .changes
                .iter()
                .find(|c| c.path == current.path)
                .ok_or(LeaseError::Conflict)?;
            if expected != &current.version
                || change
                    .before
                    .as_ref()
                    .is_none_or(|v| !apply_editor::matches_text(v, &current.text))
            {
                return Err(LeaseError::Conflict.into());
            }
            let objects = ProposalObjects::open(
                &store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| Error::Storage)?,
            )?;
            let text = apply_editor::replacement_text(&tx, &objects, change)?;
            if proposal_undos::dispatch_in(&tx, original, operation, &current.path)?
                != DispatchAdmission::NewIntent
            {
                return Err(Error::RecoveryRequired);
            }
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(UndoEditorAdmission::Dispatch(EditorDispatch {
                operation,
                expected: current,
                text,
            }))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Exact host receipt only. Never save a buffer or infer a receipt from disk.
    pub async fn confirm_editor_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        actual: EditorObservation,
    ) -> Result<StoredUndo, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let saved =
                proposal_undos::read_in(&db, original, operation)?.ok_or(LeaseError::Conflict)?;
            let file = saved
                .application
                .files
                .iter()
                .find(|f| f.path == actual.path)
                .ok_or(LeaseError::Conflict)?;
            if !matches!(file.target, ApplicationTarget::Editor { .. }) {
                return Err(LeaseError::Conflict.into());
            }
            let dispatched = file.phase == ApplicationPhase::Dispatched;
            let confirmed = (|| -> Result<StoredUndo, Error> {
                let mut tx = db
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|_| Error::Storage)?;
                let saved = proposal_undos::read_in(&tx, original, operation)?
                    .ok_or(LeaseError::Conflict)?;
                let change = saved
                    .changes
                    .iter()
                    .find(|c| c.path == actual.path)
                    .ok_or(LeaseError::Conflict)?;
                let after = change.after.as_ref().ok_or(LeaseError::Conflict)?;
                if !apply_editor::matches_text(after, &actual.text) {
                    return Err(LeaseError::Conflict.into());
                }
                proposal_undos::acknowledge_in(
                    &tx,
                    original,
                    operation,
                    &FileReceipt {
                        path: actual.path,
                        after: Some(after.clone()),
                        editor: Some(actual.version),
                    },
                )?;
                let saved = proposal_undos::read_in(&tx, original, operation)?
                    .ok_or(LeaseError::Conflict)?;
                if saved
                    .application
                    .files
                    .iter()
                    .all(|f| f.phase == ApplicationPhase::Acknowledged)
                {
                    proposal_undos::finish_in(&mut tx, original, operation)?;
                }
                let saved = proposal_undos::read_in(&tx, original, operation)?
                    .ok_or(LeaseError::Conflict)?;
                tx.commit().map_err(|_| Error::RecoveryRequired)?;
                Ok(saved)
            })();
            if dispatched && confirmed.is_err() {
                if let Ok(tx) =
                    db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    && proposal_undos::require_recovery_in(&tx, original, operation).is_ok()
                {
                    let _ = tx.commit();
                }
                return Err(Error::RecoveryRequired);
            }
            confirmed
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}
