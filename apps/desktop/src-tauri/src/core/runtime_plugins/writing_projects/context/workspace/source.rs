//! Request-local source decisions and operation leases for original Core phases.
use super::super::{Binding, Reader, Resources};
use crate::{
    agent::{
        writing_context::{ContextNeed, WritingContextError},
        writing_submission::{WritingSubmissionError, workspace::Error},
        writing_workspace::WritingWorkspaceRequest,
    },
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    runtime_plugins::{
        native::{NativeHostError, NativeProfile, ProjectOperation, package},
        writing_projects::{data_failure, failure},
    },
};
use uuid::Uuid;

fn source_failure(cause: Failure<WritingContextError>, key: &str) -> Error {
    let error = match cause.cause {
        FailureCause::Original => return WritingSubmissionError::from(cause.error).into(),
        FailureCause::NativeRejection(origin) => {
            let mut error = failure("plugin_host_source_rejected", cause.error.to_string(), key);
            error.instance_id = Some(origin.instance_id);
            error.request_id = Some(origin.native_request_id);
            error
        }
        FailureCause::SourceData(error) => data_failure(error),
        FailureCause::SourceDatabase(cause) => {
            let mut error = failure("storage_error", "工作副本来源的数据库读取失败。", key);
            error.query_database_error = Some(cause);
            error
        }
        FailureCause::SourceReadUnavailable => failure(
            "plugin_host_source_unavailable",
            cause.error.to_string(),
            key,
        ),
    };
    WritingSubmissionError::from(error).into()
}

fn native(cause: Box<NativeHostError>, operation: Uuid) -> Error {
    let mut cause = cause;
    cause.operation_id = Some(operation.to_string());
    WritingSubmissionError::from(cause).into()
}

pub(super) struct WorkspaceSource {
    binding: Binding,
    request_id: Uuid,
    #[cfg(target_os = "macos")]
    pub(super) command_scope:
        Option<crate::agent::repository::proposal_applications::ApplicationScope>,
    request: Option<WritingWorkspaceRequest>,
    editor: Option<crate::agent::writing_submission::workspace::EditorObservation>,
    reader: Option<Reader>,
    operation: Option<ProjectOperation>,
    defer_index_failure: bool,
    deferred_index_error: Option<(String, Error)>,
    checkpoint_resources: Option<super::super::super::files::checkpoints::Resources>,
    index_resources: Option<
        std::sync::Arc<crate::runtime_plugins::writing_projects::files::agent_index::Resources>,
    >,
}
impl WorkspaceSource {
    pub(super) fn new(
        binding: Binding,
        request_id: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Self {
        Self {
            request: Some(request),
            ..Self::empty(binding, request_id)
        }
    }
    pub(super) fn empty(binding: Binding, request_id: Uuid) -> Self {
        Self {
            binding,
            request_id,
            request: None,
            #[cfg(target_os = "macos")]
            command_scope: None,
            editor: None,
            reader: None,
            operation: None,
            defer_index_failure: false,
            deferred_index_error: None,
            index_resources: None,
            checkpoint_resources: None,
        }
    }
    pub(super) fn editor(
        binding: Binding,
        operation: Uuid,
        current: crate::agent::writing_submission::workspace::EditorObservation,
    ) -> Self {
        Self {
            editor: Some(current),
            ..Self::empty(binding, operation)
        }
    }
    pub(super) fn prepare_editor(
        &mut self,
        restore_id: Uuid,
        prepare: impl FnOnce(
            &crate::agent::writing_submission::workspace::EditorObservation,
            &mut Self,
        ) -> Result<
            crate::agent::writing_submission::workspace::restore_editor::PreparedEditorRestoration,
            Flow<Error, ContextNeed>,
        >,
    ) -> Result<
        crate::agent::writing_submission::workspace::EditorRestoreAdmission,
        Flow<Error, ContextNeed>,
    > {
        let current = self
            .editor
            .take()
            .expect("editor restoration retains its observation");
        match prepare(&current, self) {
            Ok(prepared) => Ok(prepared.admit(restore_id, current)),
            Err(cause) => {
                self.editor = Some(current);
                Err(cause)
            }
        }
    }
    pub(super) fn reconciliation(binding: Binding, request_id: Uuid) -> Self {
        Self {
            defer_index_failure: true,
            ..Self::empty(binding, request_id)
        }
    }
    #[cfg(target_os = "macos")]
    pub(super) fn command_checkpoint(
        &mut self,
        db: &rusqlite::Connection,
        scope: crate::agent::repository::proposal_applications::ApplicationScope,
        path: &str,
    ) -> Result<
        Option<research_workbench_native_sdk::writing_checkpoint::CheckpointRecord>,
        Flow<crate::agent::writing_workspace_commands::Error, ContextNeed>,
    > {
        self.request_id = scope.operation;
        self.command_scope = Some(scope);
        self.read_checkpoint(db, scope.manuscript, path)
            .map_err(|cause| {
                cause.map_error(|cause| {
                    crate::agent::writing_application_commands::checkpoint_source_error(
                        cause,
                        Some(scope),
                    )
                })
            })
    }
    pub(super) fn begin(&mut self) {
        if let Some(reader) = &mut self.reader {
            reader.begin();
        }
    }
    pub(super) fn capture(
        &mut self,
        db: &rusqlite::Connection,
        root: &std::path::Path,
        scope: research_agent_core::workspace::WorkspaceIdentity,
    ) -> Result<crate::agent::writing_workspace::WritingWorkspaceSnapshot, Flow<Error, ContextNeed>>
    {
        crate::agent::writing_workspace::snapshot::validate_scope(&scope)
            .map_err(|error| Flow::original(WritingSubmissionError::from(error).into()))?;
        self.ensure_reader()?;
        let manuscript = self
            .request
            .as_ref()
            .expect("capture has its original input")
            .manuscript_id;
        let files = self
            .reader
            .as_mut()
            .expect("source admitted in this capture")
            .workspace_files(db, manuscript)
            .map_err(|result| match result {
                Flow::Need(input) => Flow::Need(input),
                Flow::Fail(cause) => {
                    Flow::original(source_failure(cause, &self.binding.package_key))
                }
            })?;
        let request = self
            .request
            .take()
            .expect("input is consumed only after source decisions");
        crate::agent::writing_workspace::snapshot::from_files(root, scope, request, files)
            .map_err(|cause| Flow::original(WritingSubmissionError::from(cause).into()))
    }
    pub(super) fn with_request<T>(
        &mut self,
        attempt: impl FnOnce(&WritingWorkspaceRequest, &mut Self) -> T,
    ) -> T {
        let request = self
            .request
            .take()
            .expect("compensation retains its original input");
        let result = attempt(&request, self);
        self.request = Some(request);
        result
    }
    pub(super) fn capture_undo(
        &mut self,
        db: &rusqlite::Connection,
        root: &std::path::Path,
        scope: research_agent_core::workspace::WorkspaceIdentity,
        versions: &[crate::agent::writing_submission::workspace::UndoCheckpointVersion],
    ) -> Result<
        crate::agent::writing_submission::workspace::undo_resume::UndoPreflightSnapshot,
        Flow<Error, ContextNeed>,
    > {
        // Keep the original buffers owned through every checkpoint pause. The
        // original snapshot consumes them only after all earlier reads succeed.
        let request = self
            .request
            .take()
            .expect("undo preflight retains its original request");
        let records = crate::agent::writing_submission::workspace::undo_checkpoint::read_with(
            &request,
            versions,
            |id, path| self.read_checkpoint(db, id, path),
        );
        self.request = Some(request);
        let records = records?;
        let snapshot = self.capture(db, root, scope)?;
        Ok((snapshot, records))
    }
    fn read_checkpoint(
        &mut self,
        db: &rusqlite::Connection,
        id: Uuid,
        path: &str,
    ) -> Result<
        Option<research_workbench_native_sdk::writing_checkpoint::CheckpointRecord>,
        Flow<Error, ContextNeed>,
    > {
        self.ensure_reader()?;
        if self.checkpoint_resources.is_none() {
            let package = package::accept(
                &self.binding.catalog,
                &self.binding.package_key,
                NativeProfile::WritingProjects,
            )
            .map_err(|cause| Flow::original(native(cause, self.request_id)))?;
            let resources = super::super::super::files::checkpoints::Resources::load(
                &self.binding.catalog,
                &package,
            )
            .map_err(|cause| {
                Flow::original(native(
                    failure(cause.code, cause.message, &self.binding.package_key),
                    self.request_id,
                ))
            })?;
            self.checkpoint_resources = Some(resources);
        }
        let (decision, origin) = self
            .reader
            .as_mut()
            .expect("checkpoint reader admitted")
            .workspace_checkpoint(
                db,
                self.checkpoint_resources
                    .as_ref()
                    .expect("checkpoint resources admitted"),
                id,
                path,
            )
            .map_err(|cause| match cause {
                Flow::Need(input) => Flow::Need(input),
                Flow::Fail(cause) => {
                    Flow::original(source_failure(cause, &self.binding.package_key))
                }
            })?;
        use research_workbench_native_sdk::{
            writing_checkpoint::CheckpointDecision, writing_project::Decision,
        };
        let failure = match decision.as_ref() {
            Decision::Checkpoint { decision } => match decision.as_ref() {
                CheckpointDecision::Read { record }
                    if record.as_ref().is_none_or(|record| {
                        record.checkpoint.manuscript_id == id.to_string()
                            && record.checkpoint.relative_path == path
                    }) =>
                {
                    return Ok(record.as_deref().cloned());
                }
                CheckpointDecision::Rejected { reason } => {
                    failure(reason.code(), reason.to_string(), &self.binding.package_key)
                }
                _ => failure(
                    "plugin_host_response_mismatch",
                    "写作检查点结果与本次读取不匹配。",
                    &self.binding.package_key,
                ),
            },
            _ => failure(
                "plugin_host_response_mismatch",
                "写作检查点结果与本次读取不匹配。",
                &self.binding.package_key,
            ),
        };
        let mut failure = failure;
        failure.instance_id = Some(origin.instance_id);
        failure.request_id = Some(origin.native_request_id);
        Err(Flow::original(native(failure, self.request_id)))
    }
    fn ensure_reader(&mut self) -> Result<(), Flow<Error, ContextNeed>> {
        // Admission follows the original authorization. This reads package
        // files only; no Native wait or copy write occurs here.
        if self.reader.is_none() {
            let package = package::accept(
                &self.binding.catalog,
                &self.binding.package_key,
                NativeProfile::WritingProjects,
            )
            .map_err(|cause| Flow::original(native(cause, self.request_id)))?;
            let resources = Resources::load(&self.binding.catalog, &package).map_err(|cause| {
                Flow::original(native(
                    failure(cause.code, cause.message, &self.binding.package_key),
                    self.request_id,
                ))
            })?;
            self.reader = Some(Reader::new(resources, self.binding.package_key.clone()));
        }
        Ok(())
    }
    pub(super) fn capture_index(
        &mut self,
        path: &str,
    ) -> Result<crate::agent::writing_workspace::index::FileIndex, Flow<Error, ContextNeed>> {
        if let Some((expected, cause)) = self.deferred_index_error.take() {
            if expected != path {
                return Err(Flow::original(Error::RecoveryRequired));
            }
            return Err(Flow::original(cause));
        }
        self.ensure_reader()?;
        if self.index_resources.is_none() {
            let package = package::accept(
                &self.binding.catalog,
                &self.binding.package_key,
                NativeProfile::WritingProjects,
            )
            .map_err(|cause| Flow::original(native(cause, self.request_id)))?;
            let resources =
                crate::runtime_plugins::writing_projects::files::agent_index::Resources::load(
                    &self.binding.catalog,
                    &package,
                )
                .map_err(|cause| {
                    Flow::original(native(
                        failure(cause.code, cause.message, &self.binding.package_key),
                        self.request_id,
                    ))
                })?;
            self.index_resources = Some(std::sync::Arc::new(resources));
        }
        let (policy, origin) = self
            .reader
            .as_mut()
            .expect("admitted reader")
            .agent_file_policy(path)
            .map_err(|cause| match cause {
                Flow::Need(input) => Flow::Need(input),
                Flow::Fail(cause) => {
                    Flow::original(source_failure(cause, &self.binding.package_key))
                }
            })?;
        Ok(crate::agent::writing_workspace::index::FileIndex::Runtime(
            crate::runtime_plugins::writing_projects::files::agent_index::PreparedIndex {
                resources: self
                    .index_resources
                    .as_ref()
                    .expect("admitted index resources")
                    .clone(),
                policy,
                origin,
                operation: self.request_id,
            },
        ))
    }
    pub(super) async fn resolve(&mut self, input: ContextNeed) -> Result<(), Error> {
        let path = if self.defer_index_failure {
            match input.as_ref() {
                research_workbench_native_sdk::writing_project::Input::PrepareAgentFileChanges { relative_paths } if relative_paths.len()==1 => Some(relative_paths[0].clone()),
                _ => None,
            }
        } else {
            None
        };
        match (self.resolve_native(input).await, path) {
            (Err(cause), Some(path)) => {
                self.deferred_index_error = Some((path, cause));
                Ok(())
            }
            (result, _) => result,
        }
    }
    async fn resolve_native(&mut self, input: ContextNeed) -> Result<(), Error> {
        // The original preparation has confirmed rollback before returning Need.
        if self.operation.is_none() {
            self.operation = Some(
                self.binding
                    .service
                    .project_operation(&self.binding.package_key)
                    .await
                    .map_err(|cause| native(cause, self.request_id))?,
            );
        }
        let decision = self
            .binding
            .service
            .project_decision(self.binding.package_key.clone(), *input)
            .await
            .map_err(|cause| native(cause, self.request_id))?;
        self.reader
            .as_mut()
            .expect("pause requires its reader")
            .resolve(decision)
            .map_err(|cause| native(cause, self.request_id))
    }
}

impl crate::agent::writing_submission::workspace::restoration_source::RestorationSource
    for WorkspaceSource
{
    type Pause = ContextNeed;
    fn checkpoint(
        &mut self,
        db: &rusqlite::Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<
        Option<research_workbench_native_sdk::writing_checkpoint::CheckpointRecord>,
        Flow<Error, ContextNeed>,
    > {
        self.read_checkpoint(db, manuscript, path)
    }
    fn index(
        &mut self,
        path: &str,
    ) -> Result<crate::agent::writing_workspace::index::FileIndex, Flow<Error, ContextNeed>> {
        self.capture_index(path)
    }
}
