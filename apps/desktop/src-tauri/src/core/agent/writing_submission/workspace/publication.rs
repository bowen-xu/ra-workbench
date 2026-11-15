//! Staging transactions precede publication; content lock excludes GC throughout.
use super::*;
use crate::agent::{repository::change_sets, workspace::ProposalObjects};
use research_agent_core::workspace::FileChange;

impl WritingSubmissionService {
    /// Freeze the exact reviewed inventory. This never applies changes to sources.
    pub async fn publish_workspace_changes(
        &self,
        binding: Binding,
        expected: Vec<FileChange>,
    ) -> Result<Uuid, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_write().map_err(|_| Error::Storage)?;
            let workspace = {
                let db = store.connection().map_err(|_| Error::Storage)?;
                review::restore_review_binding(&store, &db, &binding)?
            };
            if workspace.binding != binding
                || expected.is_empty()
                || workspace.lease.changes()? != expected
            {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            let bytes = expected
                .iter()
                .flat_map(|c| [&c.before, &c.after])
                .flatten()
                .try_fold(0u64, |size, v| size.checked_add(v.byte_size))
                .ok_or(Error::Storage)?;
            store
                .ensure_content_import_capacity(bytes)
                .map_err(|_| Error::Storage)?;
            let objects = ProposalObjects::open(
                &store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| Error::Storage)?,
            )?;
            let mut failure = None;
            workspace
                .lease
                .visit_change_bytes(&expected, |change, side, bytes| {
                    let version = match side {
                        crate::agent::workspace::ChangeSide::Before => &change.before,
                        crate::agent::workspace::ChangeSide::After => &change.after,
                    }
                    .as_ref()
                    .ok_or(LeaseError::Conflict)?;
                    if let Err(error) =
                        super::object_staging::stage(&store, &objects, version, bytes)
                    {
                        failure = Some(error);
                        return Err(LeaseError::Unavailable);
                    }
                    Ok(())
                })
                .map_err(|error| failure.unwrap_or(Error::Lease(error)))?;
            publish_staged_changes(&store, &binding, &expected)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Publish bytes captured by an external immutable source (for example a
    /// Git revision or an isolated worktree) through the same ChangeSet table.
    /// This never creates a binding, applies a proposal, or mutates the
    /// manuscript; acceptance still uses the normal editor/source preflight.
    pub async fn publish_external_changes(
        &self,
        binding: Binding,
        expected: Vec<FileChange>,
        payloads: Vec<(String, Vec<u8>)>,
    ) -> Result<Uuid, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_write().map_err(|_| Error::Storage)?;
            let workspace = {
                let db = store.connection().map_err(|_| Error::Storage)?;
                review::restore_review(
                    &store,
                    &db,
                    WritingSubmissionReceipt {
                        conversation_id: binding.conversation_id,
                        request_id: binding.request_id,
                        manuscript_id: binding.manuscript_id,
                    },
                    binding.context_snapshot_id,
                )?
            };
            if workspace.binding != binding || expected.is_empty() {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            change_sets::validate_inventory(workspace.lease.manifest(), &expected)?;
            let payloads = payloads
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>();
            let bytes = expected
                .iter()
                .flat_map(|change| [&change.before, &change.after])
                .flatten()
                .try_fold(0u64, |size, version| size.checked_add(version.byte_size))
                .ok_or(Error::Storage)?;
            store
                .ensure_content_import_capacity(bytes)
                .map_err(|_| Error::Storage)?;
            let objects = ProposalObjects::open(
                &store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| Error::Storage)?,
            )?;
            let mut failure = None;
            workspace
                .lease
                .visit_baseline_change_bytes(&expected, |change, bytes| {
                    let version = change.before.as_ref().ok_or(LeaseError::Conflict)?;
                    if let Err(error) =
                        super::object_staging::stage(&store, &objects, version, bytes)
                    {
                        failure = Some(error);
                        return Err(LeaseError::Unavailable);
                    }
                    Ok(())
                })
                .map_err(|error| failure.unwrap_or(Error::Lease(error)))?;
            for change in &expected {
                let Some(version) = change.after.as_ref() else {
                    continue;
                };
                let bytes = payloads.get(&version.sha256).ok_or(LeaseError::Conflict)?;
                super::object_staging::stage(&store, &objects, version, bytes)?;
            }
            publish_staged_changes(&store, &binding, &expected)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Bounded native reconciliation. Missing/corrupt bytes remain journaled;
    /// recovery never fabricates a proposal or silently removes damaged objects.
    pub async fn recover_proposal_objects(&self) -> Result<(usize, usize), Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || super::object_staging::recover(&store))
            .await
            .map_err(|_| Error::Storage)?
    }
}

fn publish_staged_changes(
    store: &AppStore,
    binding: &Binding,
    expected: &[FileChange],
) -> Result<Uuid, Error> {
    let mut db = store.connection().map_err(|_| Error::Storage)?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
    let prepared = change_sets::publish_in(&mut tx, Uuid::new_v4(), binding, expected);
    match prepared {
        Ok(id) => match tx.commit() {
            Ok(()) if db.is_autocommit() => Ok(id),
            _ => {
                if !db.is_autocommit() {
                    let _ = db.execute_batch("rollback");
                }
                db.require_recovery();
                Err(Error::RecoveryRequired)
            }
        },
        Err(error) => match tx.rollback() {
            Ok(()) if db.is_autocommit() => Err(error.into()),
            _ => {
                db.require_recovery();
                Err(Error::RecoveryRequired)
            }
        },
    }
}

pub(crate) fn schedule_object_recovery(store: AppStore) {
    tauri::async_runtime::spawn(async move {
        let service = WritingSubmissionService::from_app_store(&store);
        loop {
            match service.recover_proposal_objects().await {
                Ok((_, unresolved)) if unresolved > 0 => {
                    eprintln!("Agent proposal objects require recovery: {unresolved}")
                }
                Err(_) => eprintln!("Agent proposal object recovery unavailable"),
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
        }
    });
}
