//! Native editor dispatch boundary. No disk fallback or renderer-facing IPC.
use super::*;
use crate::agent::{
    repository::{
        change_sets,
        proposal_applications::{
            self as applications, ApplicationPhase, ApplicationScope, ApplicationTarget,
            StoredApplication,
        },
    },
    workspace::ProposalObjects,
};
use research_agent_core::workspace::{EditorVersion, FileChange, FileVersion};
use sha2::{Digest, Sha256};

/// Supplied by the host's live editor port, not model arguments. Scope binds the manuscript.
pub struct EditorObservation {
    pub path: String,
    pub version: EditorVersion,
    pub text: String,
}
pub struct EditorDispatch {
    pub operation: Uuid,
    pub expected: EditorObservation,
    pub text: String,
}
pub enum EditorAdmission {
    Dispatch(EditorDispatch),
    Existing(StoredApplication),
}

pub(super) fn matches_text(version: &FileVersion, text: &str) -> bool {
    text.len() <= 4 * 1024 * 1024
        && version.byte_size == text.len() as u64
        && version.sha256 == format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Shared by whole-plan preflight and dispatch: unsupported editor operations
/// must fail before an earlier disk target can be applied.
pub(super) fn replacement_text(
    tx: &rusqlite::Connection,
    objects: &ProposalObjects,
    change: &FileChange,
) -> Result<String, Error> {
    let (Some(before), Some(after)) = (&change.before, &change.after) else {
        return Err(LeaseError::Conflict.into());
    };
    if before.executable != after.executable
        || before.byte_size > 4 * 1024 * 1024
        || after.byte_size > 4 * 1024 * 1024
        || before.sha256 == after.sha256
    {
        return Err(LeaseError::Conflict.into());
    }
    let object: String = tx
        .query_row(
            "select relative_path from content_objects where sha256=?1",
            [&after.sha256],
            |r| r.get(0),
        )
        .map_err(|_| Error::Storage)?;
    String::from_utf8(objects.read_registered(after, &object)?)
        .map_err(|_| LeaseError::Conflict.into())
}

impl WritingSubmissionService {
    /// A host that lost the editor or cannot confirm a dispatched edit freezes
    /// the plan. Never infer that unavailable/conflict means no write occurred.
    pub async fn defer_editor_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
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
            let file = saved
                .files
                .iter()
                .find(|f| f.path == path)
                .ok_or(LeaseError::Conflict)?;
            if !matches!(file.target, ApplicationTarget::Editor { .. }) {
                return Err(LeaseError::Conflict.into());
            }
            if file.phase == ApplicationPhase::Acknowledged {
                return Ok(saved);
            }
            if file.phase != ApplicationPhase::Dispatched {
                return Err(LeaseError::Conflict.into());
            }
            applications::require_recovery_in(&mut tx, scope)?;
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(saved)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Full-plan admission and live editor coverage precede this call. The host
    /// applies the returned edit against exactly `expected`, once intent commits.
    pub async fn prepare_editor_proposal_file(
        &self,
        scope: ApplicationScope,
        current: EditorObservation,
    ) -> Result<EditorAdmission, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _content = store.content_read().map_err(|_| Error::Storage)?;
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            let file = saved
                .files
                .iter()
                .find(|f| f.path == current.path)
                .ok_or(LeaseError::Conflict)?;
            let ApplicationTarget::Editor { expected, .. } = &file.target else {
                return Err(LeaseError::Conflict.into());
            };
            if file.phase != ApplicationPhase::Pending {
                return Ok(EditorAdmission::Existing(saved));
            }
            let proposal = change_sets::read_file_operation_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
            )?;
            let change = proposal
                .changes
                .iter()
                .find(|c| c.path == current.path)
                .ok_or(LeaseError::Conflict)?;
            let Some(before) = &change.before else {
                return Err(LeaseError::Conflict.into());
            };
            if expected != &current.version || !matches_text(before, &current.text) {
                return Err(LeaseError::Conflict.into());
            }
            let objects = ProposalObjects::open(
                &store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| Error::Storage)?,
            )?;
            let text = replacement_text(&tx, &objects, change)?;
            if applications::dispatch_in(&tx, scope, &current.path)?
                != applications::DispatchAdmission::NewIntent
            {
                return Err(Error::RecoveryRequired);
            }
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(EditorAdmission::Dispatch(EditorDispatch {
                operation: scope.operation,
                expected: current,
                text,
            }))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Host routes the operation-specific port receipt here. This validates actual
    /// UTF-8 bytes and session/revision; it neither saves nor changes the disk index.
    pub async fn confirm_editor_proposal_file(
        &self,
        scope: ApplicationScope,
        actual: EditorObservation,
    ) -> Result<StoredApplication, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let saved = applications::read_in(
                &db,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            let file = saved
                .files
                .iter()
                .find(|f| f.path == actual.path)
                .ok_or(LeaseError::Conflict)?;
            if !matches!(file.target, ApplicationTarget::Editor { .. }) {
                return Err(LeaseError::Conflict.into());
            }
            let dispatched = file.phase == ApplicationPhase::Dispatched;
            let confirmed = (|| -> Result<StoredApplication, Error> {
                let mut tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|_| Error::Storage)?;
                let proposal = change_sets::read_file_operation_in(
                    &tx,
                    scope.conversation,
                    scope.manuscript,
                    scope.proposal,
                )?;
                let change = proposal
                    .changes
                    .iter()
                    .find(|c| c.path == actual.path)
                    .ok_or(LeaseError::Conflict)?;
                let after = change.after.as_ref().ok_or(LeaseError::Conflict)?;
                if !matches_text(after, &actual.text) {
                    return Err(LeaseError::Conflict.into());
                }
                applications::acknowledge_in(
                    &tx,
                    scope,
                    &applications::FileReceipt {
                        path: actual.path,
                        after: Some(after.clone()),
                        editor: Some(actual.version),
                    },
                )?;
                let saved = applications::read_in(
                    &tx,
                    scope.conversation,
                    scope.manuscript,
                    scope.proposal,
                    scope.operation,
                )?;
                if saved
                    .files
                    .iter()
                    .all(|f| f.phase == ApplicationPhase::Acknowledged)
                {
                    applications::finish_in(&mut tx, scope)?;
                }
                let saved = applications::read_in(
                    &tx,
                    scope.conversation,
                    scope.manuscript,
                    scope.proposal,
                    scope.operation,
                )?;
                tx.commit().map_err(|_| Error::RecoveryRequired)?;
                Ok(saved)
            })();
            if dispatched && confirmed.is_err() {
                // Once an editor write may have happened, freeze further dispatch.
                // If storage is unavailable the persisted intent still requires reconciliation.
                if let Ok(mut tx) = db.transaction_with_behavior(TransactionBehavior::Immediate)
                    && applications::require_recovery_in(&mut tx, scope).is_ok()
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
