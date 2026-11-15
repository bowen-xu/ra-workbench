//! Blocking storage operation invoked only inside AgentSessionService's idle gate.
use super::*;
use crate::agent::{repository::execution_workspaces::discard as journal, workspace::DiscardState};
use sha2::{Digest, Sha256};

pub(crate) async fn execute(
    writing: WritingSubmissionService,
    request: journal::Request,
) -> Result<(), Error> {
    request.validate()?;
    let store = writing.store;
    tokio::task::spawn_blocking(move || run(&store, &request))
        .await
        .map_err(|_| Error::RecoveryRequired)?
}

fn run(store: &AppStore, request: &journal::Request) -> Result<(), Error> {
    let mut db = store.connection().map_err(|_| Error::Storage)?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let saved = journal::read(&tx, request)?;
    let baseline = if saved == Some(journal::State::Complete) {
        None
    } else {
        Some(journal::baseline(&tx, request)?)
    };
    let copies = open(store)?;
    let identity = request.identity();
    let files = copies.discard_state(&identity)?;
    if saved == Some(journal::State::Complete) {
        if files != Some(DiscardState::Complete) {
            return Err(Error::RecoveryRequired);
        }
        copies.discard(&identity)?;
        journal::complete_in(&mut tx, request)?;
        return Ok(());
    }
    if saved.is_none() && files.is_some() {
        return Err(Error::RecoveryRequired);
    }
    let baseline = baseline.ok_or(Error::RecoveryRequired)?;
    // With no filesystem journal, the original inventory must still be intact.
    // With a journal, removal may already be partial and must not be rescanned.
    let held = if files.is_none() {
        let lease = copies.restore(&identity)?;
        if lease.manifest() != &baseline {
            return Err(LeaseError::Conflict.into());
        }
        let changes = lease.changes()?;
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&changes).map_err(|_| Error::Storage)?)
        );
        if hash != request.inventory_hash {
            return Err(LeaseError::Conflict.into());
        }
        Some((lease, changes))
    } else {
        None
    };
    journal::begin_in(&mut tx, request)?;
    tx.commit().map_err(|_| Error::RecoveryRequired)?;
    drop(db);
    // Hold ownership across the database commit and parent journal publication.
    // Closing a frontend waiter cannot release this worker's lease midway.
    if let Some((lease, changes)) = held {
        lease.prepare_discard(&changes)?;
    }
    copies.discard(&identity)?;
    let mut db = store.connection().map_err(|_| Error::RecoveryRequired)?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::RecoveryRequired)?;
    journal::complete_in(&mut tx, request)?;
    tx.commit().map_err(|_| Error::RecoveryRequired)?;
    Ok(())
}

#[cfg(test)]
#[path = "discard/tests.rs"]
mod tests;
