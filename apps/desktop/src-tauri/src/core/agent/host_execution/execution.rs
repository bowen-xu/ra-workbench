use super::*;
use crate::agent::{
    process::ProcessRegistry,
    service::AgentSessionService,
    shell_tools::{
        ExecRequest, ShellHost, ShellRequest, ShellTools, workspace_host::WorkspaceShellHost,
    },
    workspace::{WorkspaceInput, WorkspaceStore},
};
use research_agent_core::{
    runner::RunFuture,
    tool_registry::OwnedToolRegistration,
    workspace::{BaselineFile, SourceVersion, WorkspaceFile, WorkspaceIdentity, WorkspaceManifest},
};
use std::sync::Arc;
use tokio::sync::OnceCell;
struct Host {
    store: AppStore,
    record: SubmissionRecord,
    manifest: WorkspaceManifest,
    revision: String,
    registry: Arc<ProcessRegistry>,
    host: OnceCell<WorkspaceShellHost>,
}
impl Host {
    async fn get(&self) -> Result<&WorkspaceShellHost, RunError> {
        self.host
            .get_or_try_init(|| async {
                let store = self.store.clone();
                let record = self.record.clone();
                let manifest = self.manifest.clone();
                let revision = self.revision.clone();
                let registry = self.registry.clone();
                tokio::task::spawn_blocking(move || {
                    {
                        let db = store.connection().map_err(|_| RunError::EventSink)?;
                        if grant(&db, &record)?.as_deref() != Some(&revision) {
                            return Err(RunError::ToolRejected);
                        }
                    }
                    let root = store.agent_workspace_directory();
                    std::fs::create_dir_all(root).map_err(|_| RunError::ToolRejected)?;
                    let root = root.canonicalize().map_err(|_| RunError::ToolRejected)?;
                    let workspaces = WorkspaceStore::open(&root, |_| Ok(()))
                        .map_err(|_| RunError::ToolRejected)?;
                    let saved: Option<String> = {
                        let db = store.connection().map_err(|_| RunError::EventSink)?;
                        db.query_row(
                            "select manifest_json from agent_host_execution_workspaces where turn_id=?1",
                            [record.turn_id.to_string()],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|_| RunError::EventSink)?
                    };
                    // Only the owned runner acquires a lease. Recovery never
                    // reruns a command whose acknowledgement was lost.
                    let lease = if let Some(saved) = saved {
                        let saved: WorkspaceManifest = serde_json::from_str(&saved)
                            .map_err(|_| RunError::EventSink)?;
                        if saved != manifest {
                            return Err(RunError::ToolRejected);
                        }
                        workspaces.restore(&manifest.identity)
                            .map_err(|_| RunError::ToolRejected)?
                    } else {
                        let inputs: Vec<WorkspaceInput> = record.input.context.iter().enumerate()
                            .map(|(index, context)| WorkspaceInput {
                                path: format!("context/{index}.txt"),
                                bytes: context.text.as_bytes().to_vec(),
                            })
                            .collect();
                        let lease = workspaces.create(manifest.clone(), inputs)
                            .map_err(|_| RunError::ToolRejected)?;
                        let db = store.connection().map_err(|_| RunError::EventSink)?;
                        db.execute(
                            "insert into agent_host_execution_workspaces(turn_id,manifest_json,grant_revision) values(?1,?2,?3)",
                            params![record.turn_id.to_string(), serde_json::to_string(&manifest).map_err(|_| RunError::EventSink)?, revision],
                        ).map_err(|_| RunError::EventSink)?;
                        lease
                    };
                    let work = lease.working_directory().map_err(|_| RunError::ToolRejected)?;
                    let read_only = crate::agent::skills::packages::freeze(&store, &record, &work)?;
                    let protected = crate::agent::workspace::sandbox::protected_paths(&store, &work)?;
                    let expected = manifest.identity;
                    let authorize = Arc::new(move |scope: &WorkspaceIdentity, _: &ShellRequest| {
                        if scope != &expected {
                            return Err(RunError::ToolRejected);
                        }
                        let db = store.connection().map_err(|_| RunError::EventSink)?;
                        if grant(&db, &record)?.as_deref() != Some(&revision) {
                            return Err(RunError::ToolRejected);
                        }
                        Ok(())
                    });
                    WorkspaceShellHost::new(lease, registry, protected, authorize)?
                        .with_read_only(read_only)
                })
                .await
                .map_err(|_| RunError::EventSink)?
            })
            .await
    }
}
impl ShellHost for Host {
    fn authorize(&self, scope: WorkspaceIdentity, request: ShellRequest) -> RunFuture<'_, ()> {
        Box::pin(async move { self.get().await?.authorize(scope, request).await })
    }
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        request: ExecRequest,
    ) -> RunFuture<'_, Uuid> {
        Box::pin(async move { self.get().await?.launch(registry, scope, request).await })
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
        (grant(&db, record).map_err(|_| HookContextError)?.as_deref() == Some(&self.revision))
            .then_some(())
            .ok_or(HookContextError)
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
            let host = self
                .get()
                .await
                .map_err(|_| ModelError::Unavailable)?
                .clone();
            let store = self.store.clone();
            let record = self.record.clone();
            let revision = self.revision.clone();
            let expected = self.manifest.identity.clone();
            crate::agent::compaction_hook_config::native::HookWorkspace::from_host(
                host,
                Arc::new(move |scope| {
                    use crate::agent::compaction_hook_process::HookCommandError;
                    if scope != &expected {
                        return Err(HookCommandError);
                    }
                    let db = store.connection().map_err(|_| HookCommandError)?;
                    if grant(&db, &record)
                        .map_err(|_| HookCommandError)?
                        .as_deref()
                        != Some(&revision)
                    {
                        return Err(HookCommandError);
                    }
                    Ok(())
                }),
            )
            .await
        })
    }
}
#[allow(dead_code, reason = "retained for shell-only fixture construction")]
pub(crate) async fn registration(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    existing: OwnedToolRegistration,
) -> Result<OwnedToolRegistration, RunError> {
    Ok(registration_with_hooks(store, record, sessions, existing)
        .await?
        .tools)
}
pub(crate) async fn registration_with_hooks(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    existing: OwnedToolRegistration,
) -> Result<HostTools, RunError> {
    let lookup = store.clone();
    let frozen = record.clone();
    let Some((revision, manifest)) = tokio::task::spawn_blocking(
        move || -> Result<Option<(String, WorkspaceManifest)>, RunError> {
            let db = lookup.connection().map_err(|_| RunError::EventSink)?;
            let Some(revision) = grant(&db, &frozen)? else {
                return Ok(None);
            };
            let files = frozen
                .input
                .context
                .iter()
                .enumerate()
                .map(|(index, context)| BaselineFile {
                    file: WorkspaceFile {
                        path: format!("context/{index}.txt"),
                        version: crate::agent::workspace::file_version(
                            context.text.as_bytes(),
                            false,
                        ),
                    },
                    source: SourceVersion {
                        resource_id: context.source_id.clone(),
                        revision: context.revision.clone(),
                        editor: None,
                    },
                })
                .collect();
            let manifest = WorkspaceManifest {
                schema_version: 1,
                identity: WorkspaceIdentity {
                    lease_id: frozen.turn_id,
                    conversation_id: frozen.conversation_id,
                    turn_id: frozen.turn_id,
                    environment_id: format!("host:{}", frozen.conversation_id),
                },
                files,
            };
            Ok(Some((revision, manifest)))
        },
    )
    .await
    .map_err(|_| RunError::EventSink)??
    else {
        return Ok(HostTools {
            tools: existing,
            workspace: None,
        });
    };
    let registry = sessions.processes().clone();
    let scope = manifest.identity.clone();
    let mapping = record
        .input
        .context
        .iter()
        .enumerate()
        .map(|(index, context)| format!("\n- {}: context/{index}.txt", context.source_id))
        .collect::<String>();
    let host = Arc::new(Host {
        store,
        record,
        manifest,
        revision,
        registry: registry.clone(),
        host: OnceCell::new(),
    });
    let shell = ShellTools::new(registry, scope, host.clone());
    let (mut definitions, executor) = existing.into_parts();
    let mut shell_definitions = shell.definitions();
    let mapping = format!(
        "\nFrozen host resources in this isolated working directory:{mapping}\nSkill packages are read-only at the executionDirectory reported by read_skill. Missing dependencies must be reported; do not modify the host resources."
    );
    for definition in &mut shell_definitions {
        if definition.name == crate::agent::shell_tools::EXEC {
            definition.description.push_str(&mapping);
        }
    }
    let combined = crate::agent::composed_tools::ComposedToolExecutor::new(
        executor,
        definitions.clone(),
        Some(Arc::new(shell)),
        shell_definitions.clone(),
    );
    definitions.extend(shell_definitions);
    Ok(HostTools {
        tools: OwnedToolRegistration::new(definitions, Arc::new(combined)),
        workspace: Some(host),
    })
}
