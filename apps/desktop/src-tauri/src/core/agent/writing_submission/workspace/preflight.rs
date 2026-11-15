//! Read-only acceptance preflight. A successful check is not a write lease;
//! the apply coordinator must revalidate when recording/executing its intent.
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceSnapshot;
use crate::agent::{
    repository::change_sets::{self, ChangeSetStatus, StoredChangeSet},
    workspace::ProposalObjects,
};
use crate::ai::structured_resource_validation::Flow;
use research_agent_core::workspace::EditorVersion;
use research_agent_core::workspace::check_source;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};

/// Frozen proposal provenance stays immutable; this checked map owns dispatch.
pub(super) struct ApplicationPreflight {
    pub targets: BTreeMap<String, Option<EditorVersion>>,
    pub proposal: StoredChangeSet,
}

enum EditorBinding {
    FrozenSource,
    CurrentSession,
}

impl WritingSubmissionService {
    /// The editor coordinator must supply all current buffers. Missing a buffer
    /// captured in the proposal conflicts rather than falling back to disk.
    pub async fn preflight_proposal(
        &self,
        conversation: Uuid,
        proposal: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredChangeSet, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_read().map_err(|_| Error::Storage)?;
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            preflight_in(&store, &tx, conversation, proposal, request)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

// Caller owns the content read lock and a consistent database transaction.
pub(super) fn preflight_in(
    store: &AppStore,
    tx: &rusqlite::Connection,
    conversation: Uuid,
    proposal: Uuid,
    request: WritingWorkspaceRequest,
) -> Result<StoredChangeSet, Error> {
    let frozen = change_sets::read_in(tx, conversation, request.manuscript_id, proposal)?;
    preflight_with_status(
        store,
        tx,
        frozen,
        request.manuscript_id,
        EditorBinding::FrozenSource,
        |status| status == ChangeSetStatus::Pending,
        |db, root, scope| static_snapshot(store, db, root, scope, request),
    )
    .map_err(Flow::into_static)
    .map(|checked| checked.proposal)
}

pub(super) fn preflight_application_in_with_snapshot<P>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    conversation: Uuid,
    proposal: Uuid,
    manuscript: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<ApplicationPreflight, Flow<Error, P>> {
    let frozen = change_sets::read_in(tx, conversation, manuscript, proposal)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    preflight_with_status(
        store,
        tx,
        frozen,
        manuscript,
        EditorBinding::CurrentSession,
        |status| status == ChangeSetStatus::Pending,
        capture,
    )
}

/// Resume preflight for an already admitted operation. It accepts only the
/// interrupted lifecycle states; the caller separately proves that every
/// journal file is still pending before reopening dispatch.
pub(super) fn preflight_resume_in_with_snapshot<P>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    conversation: Uuid,
    proposal: Uuid,
    manuscript: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<ApplicationPreflight, Flow<Error, P>> {
    let frozen = change_sets::read_file_operation_in(tx, conversation, manuscript, proposal)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    preflight_with_status(
        store,
        tx,
        frozen,
        manuscript,
        EditorBinding::CurrentSession,
        |status| {
            matches!(
                status,
                ChangeSetStatus::Applying | ChangeSetStatus::RecoveryRequired
            )
        },
        capture,
    )
}

pub(super) fn static_snapshot(
    store: &AppStore,
    db: &rusqlite::Connection,
    root: &Path,
    scope: WorkspaceIdentity,
    request: WritingWorkspaceRequest,
) -> Result<WritingWorkspaceSnapshot, Flow<Error, Infallible>> {
    store
        .snapshot_writing_workspace_in(db, root, scope, request, |_, _| Ok(()))
        .map_err(|cause| Flow::original(WritingSubmissionError::from(cause).into()))
}

fn preflight_with_status<P>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    frozen: StoredChangeSet,
    manuscript_id: Uuid,
    editor_binding: EditorBinding,
    allowed_status: impl Fn(ChangeSetStatus) -> bool,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<ApplicationPreflight, Flow<Error, P>> {
    if !allowed_status(frozen.status) {
        return Err(Flow::original(execution_workspaces::Error::Conflict.into()));
    }
    let root = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Flow::original(Error::Storage))?;
    let current = capture(
        tx,
        &root,
        WorkspaceIdentity {
            conversation_id: frozen.binding.conversation_id,
            turn_id: frozen.binding.turn_id,
            lease_id: frozen.binding.lease_id,
            environment_id: format!("manuscript:{manuscript_id}"),
        },
    )?;
    verify_snapshot(store, tx, frozen, current, root, editor_binding).map_err(Flow::original)
}

fn verify_snapshot(
    store: &AppStore,
    tx: &rusqlite::Connection,
    frozen: StoredChangeSet,
    current: WritingWorkspaceSnapshot,
    root: PathBuf,
    editor_binding: EditorBinding,
) -> Result<ApplicationPreflight, Error> {
    let objects = ProposalObjects::open(
        &store
            .content_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?,
    )?;
    let sources: std::collections::BTreeMap<_, _> = current
        .manifest
        .files
        .iter()
        .map(|f| (f.file.path.as_str(), f))
        .collect();
    let mut targets = BTreeMap::new();
    for change in &frozen.changes {
        let actual = sources.get(change.path.as_str()).copied();
        if matches!(editor_binding, EditorBinding::CurrentSession)
            && actual.is_some_and(|file| file.source.editor.is_some())
        {
            let expected = change
                .source
                .as_ref()
                .ok_or(execution_workspaces::Error::Conflict)?;
            let live = &actual.ok_or(execution_workspaces::Error::Conflict)?.source;
            // Auto-save may persist the exact buffer captured for execution.
            // Its disk revision changes, but the frozen before bytes do not.
            // A different disk body must still conflict, even if the caller
            // supplies its latest hash. FileVersion equality is checked below.
            let saved_frozen_buffer = expected.editor.is_some()
                && change
                    .before
                    .as_ref()
                    .is_some_and(|before| live.revision == format!("sha256:{}", before.sha256));
            if expected.resource_id != live.resource_id
                || (expected.revision != live.revision && !saved_frozen_buffer)
            {
                return Err(execution_workspaces::Error::Conflict.into());
            }
        } else {
            // Never fall back to disk when the proposal captured an editor.
            check_source(change, actual.map(|f| &f.source))
                .map_err(|_| execution_workspaces::Error::Conflict)?;
        }
        if change.before.as_ref() != actual.map(|f| &f.file.version) {
            return Err(execution_workspaces::Error::Conflict.into());
        }
        if change.before.is_none() {
            store
                .check_absent_writing_file(&root, frozen.binding.manuscript_id, &change.path)
                .map_err(WritingSubmissionError::from)?;
        }
        for version in [&change.before, &change.after].into_iter().flatten() {
            let path: String = tx
                .query_row(
                    "select relative_path from content_objects where sha256=?1",
                    [&version.sha256],
                    |r| r.get(0),
                )
                .map_err(|_| Error::Storage)?;
            objects.verify_registered(version, &path)?;
        }
        if actual.is_some_and(|file| file.source.editor.is_some()) {
            apply_editor::replacement_text(tx, &objects, change)?;
        }
        targets.insert(
            change.path.clone(),
            actual.and_then(|file| file.source.editor.clone()),
        );
    }
    Ok(ApplicationPreflight {
        proposal: frozen,
        targets,
    })
}
