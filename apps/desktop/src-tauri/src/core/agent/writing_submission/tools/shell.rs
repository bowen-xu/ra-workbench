use super::*;
use crate::agent::{
    process::ProcessRegistry,
    repository::execution_workspaces::{self, Binding},
    shell_tools::{
        ExecRequest, ShellHost, ShellRequest, ShellTools, workspace_host::WorkspaceShellHost,
    },
};
use research_agent_core::{runner::RunError, workspace::WorkspaceIdentity};
use std::sync::{
    RwLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::OnceCell;
#[path = "shell/transition_diagnostic.rs"]
mod transition_diagnostic;

// Acquire the lease only when the owned turn actually invokes a tool. Repeated
// start requests can therefore return their original admission without competing
// for a lease held by the already-running turn.
pub(super) struct Host {
    store: AppStore,
    binding: Binding,
    record: SubmissionRecord,
    registry: Arc<ProcessRegistry>,
    host: OnceCell<WorkspaceShellHost>,
    execution_requested: AtomicBool,
    successor: RwLock<Option<Successor>>,
    transition: tokio::sync::Mutex<()>,
    sealed: AtomicBool,
    transition_uncertain: AtomicBool,
    transition_request: RwLock<Option<crate::agent::writing_delivery_tool::SuccessorRequest>>,
}
#[derive(Clone)]
struct Successor {
    host: WorkspaceShellHost,
    binding: Binding,
    generation: u32,
    predecessor: Uuid,
}
type CurrentWorkspace = (WorkspaceShellHost, Binding, Option<(u32, Uuid)>);
impl Host {
    pub(super) fn requires_finalization(&self) -> bool {
        self.host.get().is_some() && self.execution_requested.load(Ordering::Acquire)
    }
    pub(super) async fn has_changes_for_finalization(&self) -> Result<bool, RunError> {
        let Some((host, _, _)) = self.current_existing()? else {
            return Ok(false);
        };
        host.inspect_owned(|lease| {
            lease
                .changes()
                .map(|changes| !changes.is_empty())
                .map_err(|_| RunError::ToolOutcomeUnknown)
        })
        .await
    }
    pub(super) async fn release_for_finalization(&self) -> Result<bool, RunError> {
        let Some((host, binding, generation)) = self.current_existing()? else {
            return Ok(false);
        };
        let store = self.store.clone();
        let registry = self.registry.clone();
        let released = host
            .inspect_owned(move |lease| {
                lease.changes().map_err(|_| RunError::ToolRejected)?;
                registry
                    .ensure_scope_empty(&lease.manifest().identity)
                    .map_err(|_| RunError::ToolOutcomeUnknown)?;
                let mut db = store.connection().map_err(|_| RunError::EventSink)?;
                let tx = db.transaction().map_err(|_| RunError::EventSink)?;
                if let Some((number, predecessor)) = generation {
                    execution_workspaces::generation::authorize(
                        &tx,
                        &binding,
                        number,
                        predecessor,
                        lease.manifest(),
                    )
                    .map_err(|_| RunError::ToolRejected)?;
                    execution_workspaces::generation::revoke_binding_in(
                        &tx,
                        &binding,
                        number,
                        predecessor,
                    )
                    .map_err(|_| RunError::ToolRejected)?;
                } else {
                    execution_workspaces::authorize(&tx, &binding, lease.manifest())
                        .map_err(|_| RunError::ToolRejected)?;
                    execution_workspaces::revoke_binding_in(&tx, &binding)
                        .map_err(|_| RunError::ToolRejected)?;
                }
                tx.commit().map_err(|_| RunError::EventSink)?;
                lease
                    .release_for_delivery()
                    .map_err(|_| RunError::ToolRejected)?;
                Ok(true)
            })
            .await?;
        self.sealed.store(true, Ordering::Release);
        Ok(released)
    }
    pub(super) async fn review(
        &self,
    ) -> Result<Option<super::super::workspace::WorkspaceReview>, RunError> {
        self.inspect(|service, db, binding, lease| {
            service.review_owned_workspace_binding(db, binding, lease)
        })
        .await
    }

    pub(super) async fn read(
        &self,
        expected: research_agent_core::workspace::FileChange,
    ) -> Result<Option<crate::agent::workspace::ChangeContent>, RunError> {
        self.inspect(move |service, db, binding, lease| {
            service.read_owned_workspace_change_binding(db, binding, lease, expected)
        })
        .await
    }

    async fn inspect<T: Send + 'static>(
        &self,
        read: impl FnOnce(
            &WritingSubmissionService,
            &rusqlite::Connection,
            &Binding,
            &crate::agent::workspace::WorkspaceLease,
        ) -> Result<T, super::super::workspace::Error>
        + Send
        + 'static,
    ) -> Result<Option<T>, RunError> {
        let Some((host, binding, _)) = self.current_existing()? else {
            return Ok(None);
        };
        let store = self.store.clone();
        host.inspect_owned(move |lease| {
            let mut db = store.connection().map_err(|_| RunError::EventSink)?;
            let tx = db.transaction().map_err(|_| RunError::EventSink)?;
            read(
                &WritingSubmissionService::from_app_store(&store),
                &tx,
                &binding,
                lease,
            )
            .map_err(|_| RunError::ToolRejected)
        })
        .await
        .map(Some)
    }
    async fn get(&self) -> Result<&WorkspaceShellHost, RunError> {
        self.host
            .get_or_try_init(|| async {
                let service = WritingSubmissionService::from_app_store(&self.store);
                let prepared = service
                    .restore_workspace(
                        WritingSubmissionReceipt {
                            conversation_id: self.binding.conversation_id,
                            request_id: self.binding.request_id,
                            manuscript_id: self.binding.manuscript_id,
                        },
                        self.binding.context_snapshot_id,
                    )
                    .await
                    .map_err(|_| RunError::ToolRejected)?;
                if prepared.binding != self.binding {
                    return Err(RunError::ToolRejected);
                }
                let store = self.store.clone();
                let registry = self.registry.clone();
                let record = self.record.clone();
                tokio::task::spawn_blocking(move || {
                    let work = prepared
                        .lease
                        .working_directory()
                        .map_err(|_| RunError::ToolRejected)?;
                    // Keep shell and compile tools on the same protected-root
                    // policy. In production Tauri commonly maps app data and
                    // app config to one Application Support ancestor; denying
                    // that ancestor would also deny the editable work copy.
                    let protected = super::compile_workspace::protected_paths(&store, &work)?;
                    let read_only = crate::agent::skills::packages::freeze(&store, &record, &work)?;
                    WorkspaceShellHost::from_approved_binding(
                        prepared.lease,
                        registry,
                        protected,
                        store.shared_agent_connection(),
                        prepared.binding,
                    )?
                    .with_read_only(read_only)
                })
                .await
                .map_err(|_| RunError::EventSink)?
            })
            .await
    }
    fn current_existing(&self) -> Result<Option<CurrentWorkspace>, RunError> {
        if let Some(active) = self
            .successor
            .read()
            .map_err(|_| RunError::EventSink)?
            .as_ref()
        {
            return Ok(Some((
                active.host.clone(),
                active.binding.clone(),
                Some((active.generation, active.predecessor)),
            )));
        }
        Ok(self
            .host
            .get()
            .map(|host| (host.clone(), self.binding.clone(), None)))
    }

    async fn current(&self) -> Result<CurrentWorkspace, RunError> {
        if !self.sealed.load(Ordering::Acquire) {
            if let Some(current) = self.current_existing()? {
                return Ok(current);
            }
            return Ok((self.get().await?.clone(), self.binding.clone(), None));
        }
        let _transition = self.transition.lock().await;
        if !self.sealed.load(Ordering::Acquire) {
            return self.current_existing()?.ok_or(RunError::ToolRejected);
        }
        if self.transition_uncertain.swap(true, Ordering::AcqRel) {
            return Err(self.transition_error("original_attempt_unconfirmed"));
        }
        let (_, previous, old_generation) =
            self.current_existing()?.ok_or(RunError::ToolRejected)?;
        let generation = old_generation.map_or(1, |(number, _)| number.saturating_add(1));
        let lease_id = Uuid::new_v4();
        let store = self.store.clone();
        let prior = previous.clone();
        let predecessor = tokio::task::spawn_blocking(move || {
            let db = store.connection().map_err(|_| RunError::EventSink)?;
            execution_workspaces::generation::confirmed_predecessor(
                &db, &prior, generation, lease_id,
            )
            .map_err(|_| RunError::ToolOutcomeUnknown)
        })
        .await
        .map_err(|_| RunError::EventSink)??;
        let request = crate::agent::writing_delivery_tool::SuccessorRequest {
            request_id: Uuid::new_v4(),
            scope: crate::agent::writing_workspace_commands::Scope {
                conversation_id: previous.conversation_id,
                request_id: previous.request_id,
                manuscript_id: previous.manuscript_id,
                context_snapshot_id: previous.context_snapshot_id,
            },
            turn_id: previous.turn_id,
            generation,
            predecessor_delivery_id: predecessor,
            lease_id,
        };
        *self
            .transition_request
            .write()
            .map_err(|_| RunError::EventSink)? = Some(request.clone());
        let confirmation = self
            .store
            .writing_delivery_tools()
            .request_successor(request)
            .await
            .map_err(|_| self.transition_error("file_owner_result_unconfirmed"))?;
        let binding = confirmation
            .binding()
            .map_err(|_| self.transition_error("confirmation_invalid"))?;
        let service = WritingSubmissionService::from_app_store(&self.store);
        let prepared = service
            .restore_next_workspace(
                WritingSubmissionReceipt {
                    conversation_id: binding.conversation_id,
                    request_id: binding.request_id,
                    manuscript_id: binding.manuscript_id,
                },
                binding.context_snapshot_id,
                generation,
                predecessor,
                lease_id,
            )
            .await
            .map_err(|_| self.transition_error("workspace_restore_unconfirmed"))?;
        if prepared.binding != binding {
            return Err(self.transition_error("workspace_identity_changed"));
        }
        let store = self.store.clone();
        let registry = self.registry.clone();
        let record = self.record.clone();
        let host = tokio::task::spawn_blocking(move || {
            let work = prepared
                .lease
                .working_directory()
                .map_err(|_| RunError::ToolRejected)?;
            let protected = super::compile_workspace::protected_paths(&store, &work)?;
            let read_only = crate::agent::skills::packages::freeze(&store, &record, &work)?;
            WorkspaceShellHost::from_approved_generation(
                prepared.lease,
                registry,
                protected,
                store.shared_agent_connection(),
                prepared.binding,
                generation,
                predecessor,
            )?
            .with_read_only(read_only)
        })
        .await
        .map_err(|_| self.transition_error("workspace_owner_unconfirmed"))?
        .map_err(|_| self.transition_error("workspace_owner_unconfirmed"))?;
        *self.successor.write().map_err(|_| RunError::EventSink)? = Some(Successor {
            host: host.clone(),
            binding: binding.clone(),
            generation,
            predecessor,
        });
        self.sealed.store(false, Ordering::Release);
        self.transition_uncertain.store(false, Ordering::Release);
        *self
            .transition_request
            .write()
            .map_err(|_| RunError::EventSink)? = None;
        Ok((host, binding, Some((generation, predecessor))))
    }
}
impl ShellHost for Host {
    fn scope(&self, _: WorkspaceIdentity) -> RunFuture<'_, WorkspaceIdentity> {
        Box::pin(async move {
            let (_, binding, _) = self.current().await?;
            Ok(WorkspaceIdentity {
                conversation_id: binding.conversation_id,
                turn_id: binding.turn_id,
                lease_id: binding.lease_id,
                environment_id: format!("manuscript:{}", binding.manuscript_id),
            })
        })
    }
    fn authorize(&self, scope: WorkspaceIdentity, request: ShellRequest) -> RunFuture<'_, ()> {
        self.execution_requested.store(true, Ordering::Release);
        Box::pin(async move { self.current().await?.0.authorize(scope, request).await })
    }
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        request: ExecRequest,
    ) -> RunFuture<'_, Uuid> {
        self.execution_requested.store(true, Ordering::Release);
        Box::pin(async move {
            self.current()
                .await?
                .0
                .launch(registry, scope, request)
                .await
        })
    }
}
impl crate::agent::text_tools::file_workspace::FileWorkspaceSource for Host {
    fn current_lease(&self, record: &SubmissionRecord) -> Result<Option<Uuid>, RunError> {
        if record != &self.record {
            return Err(RunError::ToolRejected);
        }
        Ok(self
            .current_existing()?
            .map(|(_, binding, _)| binding.lease_id))
    }
    fn workspace<'a>(
        &'a self,
        record: &'a SubmissionRecord,
    ) -> RunFuture<'a, crate::agent::text_tools::file_workspace::FileWorkspace> {
        Box::pin(async move {
            if record != &self.record {
                return Err(RunError::ToolRejected);
            }
            let (host, binding, generation) = self.current().await?;
            Ok(crate::agent::text_tools::file_workspace::FileWorkspace {
                host,
                binding,
                grant: generation.map_or_else(
                    crate::agent::text_tools::file_workspace::original_grant,
                    |(generation, predecessor)| {
                        crate::agent::text_tools::file_workspace::generation_grant(
                            generation,
                            predecessor,
                        )
                    },
                ),
            })
        })
    }
}
impl crate::agent::compaction_hook_config::native::HookWorkspaceSource for Host {
    fn authorize_artifact_read(
        &self,
        record: &SubmissionRecord,
    ) -> Result<(), research_agent_core::context::compaction_hooks::HookContextError> {
        use research_agent_core::context::compaction_hooks::HookContextError;
        if record != &self.record {
            return Err(HookContextError);
        }
        let db = self.store.connection().map_err(|_| HookContextError)?;
        let Some((binding, manifest)) =
            execution_workspaces::read(&db, self.binding.conversation_id, self.binding.request_id)
                .map_err(|_| HookContextError)?
        else {
            return Err(HookContextError);
        };
        if binding != self.binding {
            return Err(HookContextError);
        }
        execution_workspaces::authorize(&db, &binding, &manifest).map_err(|_| HookContextError)
    }
    fn prepare<'a>(
        &'a self,
        record: &'a SubmissionRecord,
    ) -> research_agent_core::model::ModelFuture<
        'a,
        crate::agent::compaction_hook_config::native::HookWorkspace,
    > {
        use research_agent_core::model::ModelError;
        Box::pin(async move {
            if record != &self.record {
                return Err(ModelError::Unavailable);
            }
            self.execution_requested.store(true, Ordering::Release);
            let host = self
                .get()
                .await
                .map_err(|_| ModelError::Unavailable)?
                .clone();
            let store = self.store.clone();
            let expected = self.binding.clone();
            crate::agent::compaction_hook_config::native::HookWorkspace::from_host(
                host,
                Arc::new(move |scope| {
                    use crate::agent::compaction_hook_process::HookCommandError;
                    let db = store.connection().map_err(|_| HookCommandError)?;
                    let Some((binding, manifest)) = execution_workspaces::read(
                        &db,
                        expected.conversation_id,
                        expected.request_id,
                    )
                    .map_err(|_| HookCommandError)?
                    else {
                        return Err(HookCommandError);
                    };
                    if binding != expected || &manifest.identity != scope {
                        return Err(HookCommandError);
                    }
                    execution_workspaces::authorize(&db, &binding, &manifest)
                        .map_err(|_| HookCommandError)
                }),
            )
            .await
        })
    }
}
pub(super) async fn for_submission(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
) -> Result<Option<(ShellTools, Arc<Host>)>, WritingSubmissionError> {
    let lookup = store.clone();
    let host_record = record.clone();
    let approved = tokio::task::spawn_blocking(move || {
        let db = lookup
            .connection()
            .map_err(|_| WritingSubmissionError::Unavailable)?;
        let saved = match execution_workspaces::read(&db, record.conversation_id, record.request_id)
        {
            Ok(saved) => saved,
            Err(execution_workspaces::Error::Denied) => return Ok(None),
            Err(_) => return Err(WritingSubmissionError::Unavailable),
        };
        let Some((binding, manifest)) = saved else {
            return Ok(None);
        };
        if binding.turn_id != record.turn_id
            || binding.context_snapshot_id != record.context_snapshot_id
        {
            return Err(SubmissionError::Conflict.into());
        }
        match execution_workspaces::authorize(&db, &binding, &manifest) {
            Ok(()) => Ok(Some((binding, manifest.identity))),
            Err(execution_workspaces::Error::Denied) => Ok(None),
            Err(_) => Err(WritingSubmissionError::Unavailable),
        }
    })
    .await
    .map_err(|_| WritingSubmissionError::Unavailable)??;
    Ok(approved.map(|(binding, scope)| {
        let registry = sessions.processes().clone();
        let host = Arc::new(Host {
            store,
            binding,
            record: host_record,
            registry: registry.clone(),
            host: OnceCell::new(),
            execution_requested: AtomicBool::new(false),
            successor: RwLock::new(None),
            transition: tokio::sync::Mutex::new(()),
            sealed: AtomicBool::new(false),
            transition_uncertain: AtomicBool::new(false),
            transition_request: RwLock::new(None),
        });
        (ShellTools::new(registry, scope, host.clone()), host)
    }))
}

#[cfg(test)]
#[path = "shell/tests.rs"]
mod tests;
