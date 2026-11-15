//! Review has independent read authority; it never reactivates command consent.
use super::*;
use crate::agent::workspace::ChangeContent;
use research_agent_core::workspace::{FileChange, FileVersion, WorkspaceManifest};

pub struct WorkspaceReview {
    pub binding: Binding,
    pub baseline: WorkspaceManifest,
    pub changes: Vec<FileChange>,
}
impl WritingSubmissionService {
    /// Inspect the exact original or successor lease supplied by a review
    /// request. This is read authority only; it never grants execution.
    pub async fn review_workspace_binding(
        &self,
        binding: Binding,
    ) -> Result<WorkspaceReview, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let PreparedWorkspace { binding, lease } =
                restore_review_binding(&store, &tx, &binding)?;
            Ok(WorkspaceReview {
                binding,
                baseline: lease.manifest().clone(),
                changes: lease.changes()?,
            })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Inspect an already-owned original or successor lease by its exact
    /// durable binding; no second filesystem owner is acquired.
    #[cfg(target_os = "macos")]
    pub(crate) fn review_owned_workspace_binding(
        &self,
        db: &rusqlite::Connection,
        binding: &Binding,
        lease: &WorkspaceLease,
    ) -> Result<WorkspaceReview, Error> {
        check_binding(db, binding.conversation_id, binding.manuscript_id)?;
        let baseline = execution_workspaces::read_binding_for_review(db, binding)?;
        if lease.manifest() != &baseline {
            return Err(execution_workspaces::Error::Conflict.into());
        }
        Ok(WorkspaceReview {
            binding: binding.clone(),
            baseline,
            changes: lease.changes()?,
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn read_owned_workspace_change_binding(
        &self,
        db: &rusqlite::Connection,
        binding: &Binding,
        lease: &WorkspaceLease,
        expected: FileChange,
    ) -> Result<ChangeContent, Error> {
        let review = self.review_owned_workspace_binding(db, binding, lease)?;
        let change = review
            .changes
            .into_iter()
            .find(|change| {
                change.path == expected.path
                    && change.before == expected.before
                    && change.after == expected.after
            })
            .ok_or(execution_workspaces::Error::Conflict)?;
        Ok(lease.change_content(&change)?)
    }

    /// Inspect a quiescent execution copy, including after its turn finishes.
    /// No lease or directory capability escapes this worker. Later acceptance
    /// must revalidate these hashes and the original editor/source revisions.
    pub async fn review_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
    ) -> Result<WorkspaceReview, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let PreparedWorkspace { binding, lease } =
                restore_review(&store, &tx, receipt, context_snapshot_id)?;
            let baseline = lease.manifest().clone();
            let changes = lease.changes()?;
            Ok(WorkspaceReview {
                binding,
                baseline,
                changes,
            })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(super) fn restore_review(
    store: &AppStore,
    db: &rusqlite::Connection,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
) -> Result<PreparedWorkspace, Error> {
    let (binding, baseline) = review_binding(db, receipt, context_snapshot_id)?;
    let lease = open(store)?.restore(&baseline.identity)?;
    if lease.manifest() != &baseline {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    Ok(PreparedWorkspace { binding, lease })
}

pub(super) fn restore_review_binding(
    store: &AppStore,
    db: &rusqlite::Connection,
    binding: &Binding,
) -> Result<PreparedWorkspace, Error> {
    check_binding(db, binding.conversation_id, binding.manuscript_id)?;
    let baseline = execution_workspaces::read_binding_for_review(db, binding)?;
    let lease = open(store)?.restore(&baseline.identity)?;
    if lease.manifest() != &baseline {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    Ok(PreparedWorkspace {
        binding: binding.clone(),
        lease,
    })
}

fn review_binding(
    db: &rusqlite::Connection,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    check_binding(db, receipt.conversation_id, receipt.manuscript_id)?;
    let (binding, baseline) =
        execution_workspaces::read_for_review(db, receipt.conversation_id, receipt.request_id)?
            .ok_or(execution_workspaces::Error::Conflict)?;
    if binding.manuscript_id != receipt.manuscript_id
        || binding.context_snapshot_id != context_snapshot_id
    {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    Ok((binding, baseline))
}
impl WritingSubmissionService {
    /// Read bounded before/after text for an exact reviewed change. Restoring
    /// frozen bytes is not permission to accept them into the live manuscript.
    pub async fn review_workspace_change(
        &self,
        binding: Binding,
        path: String,
        before: Option<FileVersion>,
        after: Option<FileVersion>,
    ) -> Result<ChangeContent, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let workspace = restore_review_binding(&store, &tx, &binding)?;
            let expected = workspace
                .lease
                .changes()?
                .into_iter()
                .find(|change| {
                    change.path == path && change.before == before && change.after == after
                })
                .ok_or(execution_workspaces::Error::Conflict)?;
            Ok(workspace.lease.change_content(&expected)?)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

impl WritingSubmissionService {
    /// Discover durable copies without opening directories or loading contents.
    pub async fn list_workspace_reviews(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> Result<execution_workspaces::history::Page, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            check_binding(&tx, conversation, manuscript)?;
            Ok(execution_workspaces::history::list(
                &tx,
                conversation,
                manuscript,
                cursor,
                limit,
            )?)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}
