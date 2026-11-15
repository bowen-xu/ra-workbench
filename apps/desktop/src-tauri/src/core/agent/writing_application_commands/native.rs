use super::*;
use crate::agent::writing_workspace::{EditorBuffer, WritingWorkspaceRequest};
use crate::agent::{
    repository::proposal_applications::{
        ApplicationPhase, ApplicationScope, ApplicationStatus, ApplicationTarget, StoredApplication,
    },
    writing_submission::{
        WritingSubmissionService,
        workspace::{EditorAdmission, EditorObservation},
    },
};
use research_agent_core::workspace::EditorVersion;

impl Scope {
    pub(super) fn checked(&self) -> Result<ApplicationScope, Error> {
        if [
            self.conversation_id,
            self.manuscript_id,
            self.proposal_id,
            self.operation_id,
        ]
        .iter()
        .any(Uuid::is_nil)
        {
            return Err(Error::InvalidInput);
        }
        Ok(ApplicationScope {
            conversation: self.conversation_id,
            manuscript: self.manuscript_id,
            proposal: self.proposal_id,
            operation: self.operation_id,
        })
    }
}
impl Observation {
    pub(super) fn checked(self) -> Result<EditorObservation, Error> {
        research_agent_core::workspace::validate_path(&self.path)
            .map_err(|_| Error::InvalidInput)?;
        if self.session_id.is_nil() || self.revision.is_nil() || self.text.len() > 4 * 1024 * 1024 {
            return Err(Error::InvalidInput);
        }
        Ok(EditorObservation {
            path: self.path,
            version: EditorVersion {
                session_id: self.session_id,
                revision: self.revision,
            },
            text: self.text,
        })
    }
}
pub(super) fn project(scope: Scope, saved: StoredApplication) -> Response {
    Response::State {
        schema_version: 1,
        scope,
        status: match saved.status {
            ApplicationStatus::Prepared => "prepared",
            ApplicationStatus::Applying => "applying",
            ApplicationStatus::Applied => "applied",
            ApplicationStatus::RecoveryRequired => "recoveryRequired",
            ApplicationStatus::RolledBack => "rolledBack",
        },
        files: saved
            .files
            .into_iter()
            .map(|f| File {
                path: f.path,
                phase: match f.phase {
                    ApplicationPhase::Pending => "pending",
                    ApplicationPhase::Dispatched => "dispatched",
                    ApplicationPhase::Acknowledged => "acknowledged",
                    ApplicationPhase::Restored => "restored",
                    ApplicationPhase::Conflict => "conflict",
                },
                target: match f.target {
                    ApplicationTarget::Disk => Target::Disk,
                    ApplicationTarget::Editor {
                        expected,
                        result_revision,
                    } => Target::Editor {
                        session_id: expected.session_id,
                        revision: expected.revision,
                        result_revision,
                    },
                },
            })
            .collect(),
    }
}
pub(super) fn error(value: crate::agent::writing_submission::workspace::Error) -> Error {
    if matches!(
        value,
        crate::agent::writing_submission::workspace::Error::Lease(
            crate::agent::workspace::LeaseError::Conflict
        )
    ) {
        return Error::Conflict;
    }
    crate::agent::writing_workspace_commands::error(value)
}
pub(crate) async fn find(
    store: &crate::storage::AppStore,
    request: crate::agent::writing_proposal_commands::ProposalRequest,
) -> Result<Option<Response>, Error> {
    request.validate()?;
    let saved = WritingSubmissionService::from_app_store(store)
        .find_proposal_application(
            request.conversation_id,
            request.manuscript_id,
            request.proposal_id,
        )
        .await
        .map_err(error)?;
    Ok(saved.map(|saved| {
        project(
            Scope {
                conversation_id: request.conversation_id,
                manuscript_id: request.manuscript_id,
                proposal_id: request.proposal_id,
                operation_id: saved.operation_id,
            },
            saved,
        )
    }))
}
pub(crate) async fn execute(
    store: &crate::storage::AppStore,
    request: Request,
) -> Result<Response, Error> {
    let service = WritingSubmissionService::from_app_store(store);
    let scope = match &request {
        Request::Begin { scope, .. }
        | Request::Resume { scope, .. }
        | Request::Read { scope }
        | Request::PrepareEditor { scope, .. }
        | Request::ConfirmEditor { scope, .. }
        | Request::DeferEditor { scope, .. }
        | Request::ApplyDisk { scope, .. }
        | Request::ReconcileDisk { scope, .. }
        | Request::RollbackDisk { scope }
        | Request::PrepareRollback { scope, .. }
        | Request::RollbackRemainder { scope, .. } => scope.clone(),
    };
    let checked = scope.checked()?;
    let source_error = |cause| {
        crate::agent::writing_workspace_commands::source_error(
            cause,
            checked.conversation,
            checked.manuscript,
            checked.operation,
            error,
        )
    };
    match request {
        Request::PrepareRollback {
            buffers,
            checkpoint_versions,
            ..
        } => {
            let request = checked_buffers(scope.manuscript_id, buffers)?;
            return Ok(project(
                scope,
                service
                    .prepare_proposal_rollback_with_checkpoints(
                        checked,
                        request,
                        checkpoint_versions,
                    )
                    .await
                    .map_err(source_error)?,
            ));
        }
        Request::RollbackRemainder {
            buffers,
            checkpoint_versions,
            ..
        } => {
            let result = if let Some(buffers) = buffers {
                let request = checked_buffers(scope.manuscript_id, buffers)?;
                service
                    .rollback_proposal_remainder_with_checkpoints(
                        checked,
                        request,
                        checkpoint_versions,
                    )
                    .await
            } else {
                if !checkpoint_versions.is_empty() {
                    return Err(Error::InvalidInput);
                }
                service.rollback_proposal_remainder(checked).await
            }
            .map_err(source_error)?;
            return Ok(project(scope, result));
        }
        Request::RollbackDisk { .. } => {
            return Ok(project(
                scope,
                service
                    .rollback_disk_proposal(checked)
                    .await
                    .map_err(source_error)?,
            ));
        }
        Request::ReconcileDisk { path, .. } => {
            research_agent_core::workspace::validate_path(&path)
                .map_err(|_| Error::InvalidInput)?;
            return Ok(project(
                scope,
                service
                    .reconcile_disk_proposal_file(checked, path)
                    .await
                    .map_err(source_error)?,
            ));
        }
        Request::ApplyDisk { path, .. } => {
            research_agent_core::workspace::validate_path(&path)
                .map_err(|_| Error::InvalidInput)?;
            return Ok(project(
                scope,
                service
                    .apply_disk_proposal_file(checked, path)
                    .await
                    .map_err(source_error)?,
            ));
        }
        Request::Read { .. } => {}
        Request::Begin { buffers, .. } => {
            if buffers.len() > 20_000 {
                return Err(Error::InvalidInput);
            }
            service
                .begin_proposal_application(
                    scope.conversation_id,
                    scope.proposal_id,
                    scope.operation_id,
                    WritingWorkspaceRequest {
                        manuscript_id: scope.manuscript_id,
                        buffers: buffers
                            .into_iter()
                            .map(|b| EditorBuffer {
                                path: b.path,
                                expected_disk_sha256: b.expected_disk_sha256,
                                text: b.text,
                                editor: EditorVersion {
                                    session_id: b.session_id,
                                    revision: b.revision,
                                },
                            })
                            .collect(),
                    },
                )
                .await
                .map_err(source_error)?;
        }
        Request::Resume { buffers, .. } => {
            if buffers.len() > 20_000 {
                return Err(Error::InvalidInput);
            }
            let manuscript_id = scope.manuscript_id;
            return Ok(project(
                scope,
                service
                    .resume_proposal_application(
                        checked,
                        WritingWorkspaceRequest {
                            manuscript_id,
                            buffers: buffers
                                .into_iter()
                                .map(|b| EditorBuffer {
                                    path: b.path,
                                    expected_disk_sha256: b.expected_disk_sha256,
                                    text: b.text,
                                    editor: EditorVersion {
                                        session_id: b.session_id,
                                        revision: b.revision,
                                    },
                                })
                                .collect(),
                        },
                    )
                    .await
                    .map_err(source_error)?,
            ));
        }
        Request::PrepareEditor { current, .. } => {
            match service
                .prepare_editor_proposal_file(checked, current.checked()?)
                .await
                .map_err(error)?
            {
                EditorAdmission::Existing(saved) => return Ok(project(scope, saved)),
                EditorAdmission::Dispatch(edit) => {
                    return Ok(Response::Edit {
                        schema_version: 1,
                        scope,
                        expected: Observation {
                            path: edit.expected.path,
                            session_id: edit.expected.version.session_id,
                            revision: edit.expected.version.revision,
                            text: edit.expected.text,
                        },
                        text: edit.text,
                    });
                }
            }
        }
        Request::ConfirmEditor { actual, .. } => {
            return Ok(project(
                scope,
                service
                    .confirm_editor_proposal_file(checked, actual.checked()?)
                    .await
                    .map_err(error)?,
            ));
        }
        Request::DeferEditor { path, .. } => {
            research_agent_core::workspace::validate_path(&path)
                .map_err(|_| Error::InvalidInput)?;
            return Ok(project(
                scope,
                service
                    .defer_editor_proposal_file(checked, path)
                    .await
                    .map_err(error)?,
            ));
        }
    }
    Ok(project(
        scope,
        service
            .read_proposal_application(
                checked.conversation,
                checked.manuscript,
                checked.proposal,
                checked.operation,
            )
            .await
            .map_err(error)?,
    ))
}

fn checked_buffers(
    manuscript_id: Uuid,
    buffers: Vec<Buffer>,
) -> Result<WritingWorkspaceRequest, Error> {
    if buffers.len() > 20_000 {
        return Err(Error::InvalidInput);
    }
    let mut paths = std::collections::BTreeSet::new();
    let buffers = buffers
        .into_iter()
        .map(|b| {
            research_agent_core::workspace::validate_path(&b.path)
                .map_err(|_| Error::InvalidInput)?;
            if !paths.insert(b.path.clone())
                || b.session_id.is_nil()
                || b.revision.is_nil()
                || b.text.len() > 4 * 1024 * 1024
                || b.expected_disk_sha256.len() != 64
                || !b
                    .expected_disk_sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(Error::InvalidInput);
            }
            Ok(EditorBuffer {
                path: b.path,
                expected_disk_sha256: b.expected_disk_sha256,
                text: b.text,
                editor: EditorVersion {
                    session_id: b.session_id,
                    revision: b.revision,
                },
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(WritingWorkspaceRequest {
        manuscript_id,
        buffers,
    })
}
