//! Fresh reverse inventory for completed applications, without write authority.
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceSnapshot;
use crate::agent::{
    repository::{change_sets, proposal_applications as applications},
    workspace::ProposalObjects,
};
use crate::ai::structured_resource_validation::Flow;
use applications::{ApplicationPhase, ApplicationScope, ApplicationStatus, ApplicationTarget};
use research_agent_core::workspace::{FileChange, FileChangeKind};
use std::path::Path;

impl WritingSubmissionService {
    /// Caller owns complete live-buffer coverage. This preview is not an undo
    /// admission: a later operation must freeze and revalidate this new baseline.
    pub async fn preview_proposal_undo(
        &self,
        scope: ApplicationScope,
        request: WritingWorkspaceRequest,
    ) -> Result<Vec<FileChange>, Error> {
        if request.manuscript_id != scope.manuscript {
            return Err(LeaseError::Conflict.into());
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_read().map_err(|_| Error::Storage)?;
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            preview_in(&store, &tx, scope, request)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(super) fn preview_in(
    store: &AppStore,
    tx: &rusqlite::Connection,
    scope: ApplicationScope,
    request: WritingWorkspaceRequest,
) -> Result<Vec<FileChange>, Error> {
    preview_with_capture(
        store,
        tx,
        scope,
        request.manuscript_id,
        |db, root, identity| preflight::static_snapshot(store, db, root, identity, request),
    )
    .map_err(Flow::into_static)
}

pub(crate) fn preview_with_capture<P>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    scope: ApplicationScope,
    manuscript: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<Vec<FileChange>, Flow<Error, P>> {
    let (applied, proposal, root) = (|| -> Result<_, Error> {
        if manuscript != scope.manuscript {
            return Err(LeaseError::Conflict.into());
        }

        let applied = applications::read_in(
            tx,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )?;
        let proposal = change_sets::read_file_operation_in(
            tx,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
        )?;
        if applied.status != ApplicationStatus::Applied
            || proposal.status != change_sets::ChangeSetStatus::Applied
            || applied
                .files
                .iter()
                .any(|file| file.phase != ApplicationPhase::Acknowledged)
        {
            return Err(LeaseError::Conflict.into());
        }
        let root = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?;

        Ok((applied, proposal, root))
    })()
    .map_err(Flow::original)?;
    let current = capture(
        tx,
        &root,
        WorkspaceIdentity {
            conversation_id: scope.conversation,
            turn_id: proposal.binding.turn_id,
            lease_id: proposal.binding.lease_id,
            environment_id: format!("manuscript:{}", scope.manuscript),
        },
    )?;
    (|| -> Result<_, Error> {
        let sources: std::collections::BTreeMap<_, _> = current
            .manifest
            .files
            .iter()
            .map(|file| (file.file.path.as_str(), file))
            .collect();
        let objects = ProposalObjects::open(
            &store
                .content_directory()
                .canonicalize()
                .map_err(|_| Error::Storage)?,
        )?;
        let mut inverse = Vec::with_capacity(proposal.changes.len());
        for change in proposal.changes {
            let actual = sources.get(change.path.as_str()).copied();
            if change.after.as_ref() != actual.map(|file| &file.file.version) {
                return Err(LeaseError::Conflict.into());
            }
            let target = applied
                .files
                .iter()
                .find(|file| file.path == change.path)
                .ok_or(LeaseError::Conflict)?;
            if matches!(target.target, ApplicationTarget::Editor { .. })
                && actual.is_none_or(|file| file.source.editor.is_none())
            {
                // An unavailable editor is not proof that disk contains its effect.
                return Err(LeaseError::Conflict.into());
            }
            if change.after.is_none() {
                store
                    .check_absent_writing_file(&root, scope.manuscript, &change.path)
                    .map_err(WritingSubmissionError::from)?;
            }
            for version in [&change.before, &change.after].into_iter().flatten() {
                let path: String = tx
                    .query_row(
                        "select relative_path from content_objects where sha256=?1",
                        [&version.sha256],
                        |row| row.get(0),
                    )
                    .map_err(|_| Error::Storage)?;
                objects.verify_registered(version, &path)?;
            }
            let reversed = FileChange {
                path: change.path,
                before: change.after,
                after: change.before,
                source: actual.map(|file| file.source.clone()),
                kind: match change.kind {
                    FileChangeKind::Added => FileChangeKind::Deleted,
                    FileChangeKind::Deleted => FileChangeKind::Added,
                    FileChangeKind::Modified => FileChangeKind::Modified,
                },
            };
            if actual.is_some_and(|file| file.source.editor.is_some()) {
                apply_editor::replacement_text(tx, &objects, &reversed)?;
            }
            inverse.push(reversed);
        }
        Ok(inverse)
    })()
    .map_err(Flow::original)
}
