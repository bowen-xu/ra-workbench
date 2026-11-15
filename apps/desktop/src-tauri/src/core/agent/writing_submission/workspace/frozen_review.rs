//! Review frozen proposals after execution copies change or are removed.
use super::*;
use crate::agent::{
    repository::change_sets::{self, StoredChangeSet},
    workspace::{ChangeContent, ProposalObjects},
};
use research_agent_core::workspace::FileVersion;

/// Review and delivery provenance may use an admitted file journal after its
/// chat leaves. A denied chat lookup never grants first-application authority.
fn read_frozen(
    db: &rusqlite::Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
) -> Result<StoredChangeSet, execution_workspaces::Error> {
    match change_sets::read_in(db, conversation, manuscript, proposal) {
        Err(execution_workspaces::Error::Denied) => {
            change_sets::read_file_operation_in(db, conversation, manuscript, proposal)
        }
        result => result,
    }
}

impl WritingSubmissionService {
    pub async fn reject_proposal(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        proposal: Uuid,
    ) -> Result<StoredChangeSet, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let frozen = change_sets::reject_in(&tx, conversation, manuscript, proposal)?;
            // No files or content references change. An uncertain commit can be
            // resolved by reading or retrying this same proposal identity.
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(frozen)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn review_proposal(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        proposal: Uuid,
    ) -> Result<StoredChangeSet, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            Ok(read_frozen(&tx, conversation, manuscript, proposal)?)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn review_proposal_file(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        proposal: Uuid,
        path: String,
    ) -> Result<ChangeContent, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_read().map_err(|_| Error::Storage)?;
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let frozen = read_frozen(&tx, conversation, manuscript, proposal)?;
            let change = frozen
                .changes
                .into_iter()
                .find(|c| c.path == path)
                .ok_or(LeaseError::Conflict)?;
            let objects = ProposalObjects::open(
                &store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| Error::Storage)?,
            )?;
            let read = |version: &FileVersion| {
                let path: String = tx
                    .query_row(
                        "select relative_path from content_objects where sha256=?1",
                        [&version.sha256],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::Storage)?;
                objects
                    .review_registered(version, &path)
                    .map_err(Error::from)
            };
            let before = change.before.as_ref().map(read).transpose()?;
            let after = change.after.as_ref().map(read).transpose()?;
            Ok(ChangeContent {
                change,
                before,
                after,
            })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}
