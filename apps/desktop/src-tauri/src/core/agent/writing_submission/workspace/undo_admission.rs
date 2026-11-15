//! Original undo admission transactions, with a request-local source pause.
use super::undo_resume::UndoPreflightSnapshot;
use super::*;
use crate::agent::repository::{
    proposal_applications::{ApplicationScope, ApplicationStatus},
    proposal_undos::{self, StoredUndo},
};
use crate::ai::structured_resource_validation::Flow;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Prepare,
    Start,
    Resume,
}

#[derive(Clone)]
pub(crate) struct Admission {
    pub original: ApplicationScope,
    pub operation: Uuid,
    pub kind: Kind,
    pub checkpoints: Vec<UndoCheckpointVersion>,
}

pub(crate) fn admit<P>(
    store: &AppStore,
    original: ApplicationScope,
    operation: Uuid,
    kind: Kind,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<UndoPreflightSnapshot, Flow<Error, P>>,
) -> Result<StoredUndo, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    let saved = proposal_undos::read_in(&tx, original, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    let saved = match (kind, saved) {
        (Kind::Prepare, Some(saved)) => return Ok(saved),
        (Kind::Start, Some(saved)) if saved.application.status != ApplicationStatus::Prepared => {
            return Ok(saved);
        }
        (Kind::Resume, Some(saved))
            if saved.application.status != ApplicationStatus::RecoveryRequired =>
        {
            return Ok(saved);
        }
        (_, saved) => saved,
    };
    let result = (|| match kind {
        Kind::Prepare => {
            let changes = undo_preview::preview_with_capture(
                store,
                &tx,
                original,
                original.manuscript,
                |db, root, identity| capture(db, root, identity).map(|(snapshot, _)| snapshot),
            )?;
            proposal_undos::begin_in(&mut tx, original, operation, &changes)
                .map_err(|cause| Flow::original(Error::from(cause)))
        }
        Kind::Start | Kind::Resume => {
            let saved = saved.ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            undo_resume::preflight_with_capture(store, &mut tx, original, &saved, capture)?;
            let transition = match kind {
                Kind::Start => proposal_undos::start_in,
                _ => proposal_undos::resume_in,
            };
            transition(&tx, original, operation)
                .map_err(|cause| Flow::original(Error::from(cause)))?;
            proposal_undos::read_in(&tx, original, operation)
                .map_err(|cause| Flow::original(Error::from(cause)))?
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))
        }
    })();
    match result {
        Ok(saved) => {
            tx.commit()
                .map_err(|_| Flow::original(Error::RecoveryRequired))?;
            Ok(saved)
        }
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            Err(Flow::Need(input))
        }
        Err(cause) => Err(cause),
    }
}
