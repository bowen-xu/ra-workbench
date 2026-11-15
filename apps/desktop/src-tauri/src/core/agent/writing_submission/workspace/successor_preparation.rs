//! Original successor intent, private copy creation and confirmation settlement.
use super::preparation::Attempt;
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceSnapshot;
use crate::ai::structured_resource_validation::Flow;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct Successor {
    pub generation: u32,
    pub predecessor: Uuid,
    pub lease_id: Uuid,
}

pub(crate) fn prepare<P>(
    store: &AppStore,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    successor: Successor,
    manuscript_id: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<Attempt<P>, Error> {
    let Successor {
        generation,
        predecessor,
        lease_id,
    } = successor;
    if lease_id.is_nil() || manuscript_id != receipt.manuscript_id {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    let root = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Error::Storage)?;
    let captured = consent::consent_with_source(store, |tx| {
        let scope = (|| -> Result<_, Error> {
            check_binding(tx, receipt.conversation_id, receipt.manuscript_id)?;
            let record = read(tx, receipt.conversation_id, receipt.request_id)
                .map_err(WritingSubmissionError::from)?
                .ok_or(execution_workspaces::Error::Conflict)?;
            if record.context_snapshot_id != context_snapshot_id {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            validate_scope(&record, receipt.manuscript_id)?;
            let exists: bool = tx
                .query_row(
                    "select exists(select 1 from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and (generation=?3 or lease_id=?4))",
                    params![
                        receipt.conversation_id.to_string(),
                        receipt.request_id.to_string(),
                        generation,
                        lease_id.to_string()
                    ],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Storage)?;
            if exists {
                return Err(Error::RecoveryRequired);
            }
            Ok(WorkspaceIdentity {
                conversation_id: receipt.conversation_id,
                turn_id: record.turn_id,
                environment_id: format!("manuscript:{}", receipt.manuscript_id),
                lease_id,
            })
        })()
        .map_err(Flow::original)?;
        let snapshot = capture(tx, &root, scope)?;
        let binding = execution_workspaces::generation::prepare_in(
            tx,
            receipt.request_id,
            receipt.manuscript_id,
            generation,
            predecessor,
            &snapshot.manifest,
        )
        .map_err(|cause| Flow::original(Error::from(cause)))?;
        Ok((snapshot, binding))
    });
    let (snapshot, binding) = match captured {
        Ok(value) => value,
        Err(Flow::Need(input)) => return Ok(Attempt::Need(input)),
        Err(Flow::Fail(cause)) => return Err(cause.error),
    };
    // Never discard a possibly created copy on an uncertain failure.
    let expected_manifest = snapshot.manifest.clone();
    let copies = open(store).map_err(|_| Error::RecoveryRequired)?;
    let lease = snapshot
        .create(&copies)
        .map_err(|_| Error::RecoveryRequired)?;
    consent::consent_transaction(store, |tx| {
        if lease.manifest() != &expected_manifest {
            return Err(execution_workspaces::Error::Conflict.into());
        }
        execution_workspaces::generation::finish_in(tx, generation, predecessor, &binding)?;
        Ok(())
    })
    .map_err(|_| Error::RecoveryRequired)?;
    Ok(Attempt::Ready(Box::new(PreparedWorkspace {
        binding,
        lease,
    })))
}
