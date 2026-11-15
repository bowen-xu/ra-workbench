//! Keep the selected native lease and its durable permission guard through file I/O.
use super::*;
use crate::agent::{repository::submissions, writing_submission::WritingSubmissionService};
use execution_workspaces::Binding;

impl FileBackend {
    /// Identify the native owner even after its write grant has been withdrawn.
    /// This is review metadata, not authorization to execute or modify files.
    pub(in crate::agent::text_tools) async fn delivery_binding(&self) -> Result<Binding, String> {
        #[cfg(target_os = "macos")]
        if let Some(source) = &self.workspace {
            let owned = source
                .workspace(&self.record)
                .await
                .map_err(|error| error.to_string())?;
            let expected = owned.binding.clone();
            let verified = expected.clone();
            owned
                .host
                .inspect_owned(move |lease| {
                    if lease.manifest().identity.conversation_id != verified.conversation_id
                        || lease.manifest().identity.turn_id != verified.turn_id
                        || lease.manifest().identity.lease_id != verified.lease_id
                    {
                        return Err(research_agent_core::runner::RunError::ToolRejected);
                    }
                    Ok(())
                })
                .await
                .map_err(|error| error.to_string())?;
            return Ok(expected);
        }
        let db = self.store.connection().map_err(|error| error.to_string())?;
        let (binding, _) = execution_workspaces::read_for_review(
            &db,
            self.record.conversation_id,
            self.record.request_id,
        )
        .map_err(|error| error.to_string())?
        .ok_or("no execution workspace")?;
        Ok(binding)
    }
    pub(in crate::agent::text_tools) async fn with_lease<T: Send + 'static>(
        &self,
        operation: impl FnOnce(
            &Self,
            &mut Connection,
            &mut WorkspaceLease,
            &Binding,
            &WorkspaceGrant,
        ) -> Result<T, String>
        + Send
        + 'static,
    ) -> Result<T, String> {
        let mut backend = self.clone();
        // Keep admission errors deferred until path authorization consumes
        // the source, after the original lease and journal checks.
        backend.scope =
            crate::agent::writing_resources::native_scope_for_context(&self.store, &self.record)
                .await;
        let access =
            move |lease: &mut WorkspaceLease, expected: &Binding, grant: &WorkspaceGrant| {
                // read_text does not check execution/release, and apply_text_operations
                // checks quiescence only after writing. Check both before any I/O and
                // hold the same mutex against process launch and delivery handoff.
                lease
                    .working_directory()
                    .map_err(|error| error.to_string())?;
                let mut db = backend
                    .store
                    .connection()
                    .map_err(|error| error.to_string())?;
                if expected.conversation_id != backend.record.conversation_id
                    || expected.request_id != backend.record.request_id
                    || expected.turn_id != backend.record.turn_id
                    || expected.context_snapshot_id != backend.record.context_snapshot_id
                    || submissions::read(
                        &db,
                        backend.record.conversation_id,
                        backend.record.request_id,
                    )
                    .map_err(|error| error.to_string())?
                    .as_ref()
                        != Some(&backend.record)
                {
                    return Err("workspace binding changed".into());
                }
                match grant {
                    WorkspaceGrant::Original => {
                        let (binding, manifest) = execution_workspaces::read(
                            &db,
                            expected.conversation_id,
                            expected.request_id,
                        )
                        .map_err(|error| error.to_string())?
                        .ok_or("no approved execution workspace")?;
                        if &binding != expected || &manifest != lease.manifest() {
                            return Err("workspace binding changed".into());
                        }
                        execution_workspaces::authorize(&db, &binding, &manifest)
                            .map_err(|error| error.to_string())?;
                    }
                    #[cfg(target_os = "macos")]
                    WorkspaceGrant::Generation {
                        generation,
                        predecessor,
                    } => {
                        execution_workspaces::generation::authorize(
                            &db,
                            expected,
                            *generation,
                            *predecessor,
                            lease.manifest(),
                        )
                        .map_err(|error| error.to_string())?;
                    }
                }
                lease.changes().map_err(|error| error.to_string())?;
                operation(&backend, &mut db, lease, expected, grant)
            };
        #[cfg(target_os = "macos")]
        if let Some(source) = &self.workspace {
            // A supplied owner is authoritative: denial never falls back to a
            // second restore or a different grant.
            let super::super::file_workspace::FileWorkspace {
                host,
                binding,
                grant,
            } = source
                .workspace(&self.record)
                .await
                .map_err(|error| error.to_string())?;
            return host
                .with_owned_lease_mut(move |lease| access(lease, &binding, &grant))
                .await
                .map_err(|error| error.to_string())?;
        }
        let mut prepared = WritingSubmissionService::from_app_store(&self.store)
            .restore_workspace(self.receipt()?, self.record.context_snapshot_id)
            .await
            .map_err(|error| error.to_string())?;
        tokio::task::spawn_blocking(move || {
            access(
                &mut prepared.lease,
                &prepared.binding,
                &WorkspaceGrant::Original,
            )
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub(in crate::agent::text_tools) async fn changes_empty(&self) -> Result<bool, String> {
        self.with_lease(|_, _, lease, _, _| {
            lease
                .changes()
                .map(|changes| changes.is_empty())
                .map_err(|error| error.to_string())
        })
        .await
    }

    pub(in crate::agent::text_tools) async fn seal_for_delivery(&self) -> Result<(), String> {
        self.with_lease(|_, db, lease, binding, grant| {
            let tx = db.transaction().map_err(|error| error.to_string())?;
            match grant {
                WorkspaceGrant::Original => execution_workspaces::revoke_binding_in(&tx, binding),
                #[cfg(target_os = "macos")]
                WorkspaceGrant::Generation {
                    generation,
                    predecessor,
                } => execution_workspaces::generation::revoke_binding_in(
                    &tx,
                    binding,
                    *generation,
                    *predecessor,
                ),
            }
            .map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
            lease
                .release_for_delivery()
                .map_err(|error| error.to_string())
        })
        .await
    }
}
