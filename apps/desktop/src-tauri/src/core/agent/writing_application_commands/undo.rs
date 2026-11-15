//! Independent undo commands; an original application response cannot authorize undo.
use super::*;
#[path = "undo/discovery.rs"]
pub(crate) mod discovery;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub scope: Scope,
    pub undo_id: Uuid,
    pub command: Command,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    Read {},
    Prepare {
        buffers: Vec<Buffer>,
    },
    Start {
        buffers: Vec<Buffer>,
    },
    Resume {
        buffers: Vec<Buffer>,
        #[serde(default, rename = "checkpointVersions")]
        checkpoint_versions:
            Vec<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
    RollbackDisk {
        buffers: Vec<Buffer>,
        #[serde(default, rename = "checkpointVersions")]
        checkpoint_versions:
            Vec<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
    PrepareRollback {
        buffers: Vec<Buffer>,
        #[serde(default, rename = "checkpointVersions")]
        checkpoint_versions:
            Vec<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
    Defer {},
    CancelPending {},
    ApplyDisk {
        path: String,
    },
    ReconcileDisk {
        path: String,
    },
    PrepareEditor {
        current: Observation,
    },
    ConfirmEditor {
        actual: Observation,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub schema_version: u8,
    pub original_scope: Scope,
    pub undo_id: Uuid,
    /// Inner operation identity is the undo ID, never the original apply ID.
    pub result: Option<super::Response>,
}

#[cfg(target_os = "macos")]
pub(crate) async fn execute(
    store: &crate::storage::AppStore,
    request: Request,
) -> Result<Response, Error> {
    use super::native::{error, project};
    use crate::agent::writing_submission::{
        WritingSubmissionService, workspace::UndoEditorAdmission,
    };
    use crate::agent::writing_workspace::{EditorBuffer, WritingWorkspaceRequest};
    use research_agent_core::workspace::EditorVersion;
    let Request {
        scope,
        undo_id,
        command,
    } = request;
    let original = scope.checked()?;
    if undo_id.is_nil() || undo_id == original.operation {
        return Err(Error::InvalidInput);
    }
    let undo_scope = Scope {
        operation_id: undo_id,
        ..scope.clone()
    };
    let service = WritingSubmissionService::from_app_store(store);
    let source_error = |cause| {
        crate::agent::writing_workspace_commands::source_error(
            cause,
            original.conversation,
            original.manuscript,
            undo_id,
            error,
        )
    };
    let workspace = |buffers: Vec<Buffer>| -> Result<WritingWorkspaceRequest, Error> {
        if buffers.len() > 20_000 {
            return Err(Error::InvalidInput);
        }
        Ok(WritingWorkspaceRequest {
            manuscript_id: original.manuscript,
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
        })
    };
    let checked_path = |path: String| -> Result<String, Error> {
        research_agent_core::workspace::validate_path(&path).map_err(|_| Error::InvalidInput)?;
        Ok(path)
    };
    let saved = match command {
        Command::Read {} => service
            .read_proposal_undo(original, undo_id)
            .await
            .map_err(error)?,
        Command::Prepare { buffers } => Some(
            service
                .prepare_proposal_undo(original, undo_id, workspace(buffers)?)
                .await
                .map_err(source_error)?,
        ),
        Command::Start { buffers } => Some(
            service
                .start_proposal_undo(original, undo_id, workspace(buffers)?)
                .await
                .map_err(source_error)?,
        ),
        Command::Resume {
            buffers,
            checkpoint_versions,
        } => Some(
            service
                .resume_proposal_undo_with_checkpoints(
                    original,
                    undo_id,
                    workspace(buffers)?,
                    checkpoint_versions,
                )
                .await
                .map_err(source_error)?,
        ),
        Command::PrepareRollback {
            buffers,
            checkpoint_versions,
        } => Some(
            service
                .prepare_proposal_undo_rollback_with_checkpoints(
                    original,
                    undo_id,
                    workspace(buffers)?,
                    checkpoint_versions,
                )
                .await
                .map_err(source_error)?,
        ),
        Command::RollbackDisk {
            buffers,
            checkpoint_versions,
        } => Some(
            service
                .rollback_disk_proposal_undo_with_checkpoints(
                    original,
                    undo_id,
                    workspace(buffers)?,
                    checkpoint_versions,
                )
                .await
                .map_err(source_error)?,
        ),
        Command::Defer {} => Some(
            service
                .defer_proposal_undo(original, undo_id)
                .await
                .map_err(error)?,
        ),
        Command::CancelPending {} => Some(
            service
                .cancel_pending_proposal_undo(original, undo_id)
                .await
                .map_err(error)?,
        ),
        Command::ApplyDisk { path } => Some(
            service
                .apply_disk_undo_file(original, undo_id, checked_path(path)?)
                .await
                .map_err(source_error)?,
        ),
        Command::ReconcileDisk { path } => Some(
            service
                .reconcile_disk_undo_file(original, undo_id, checked_path(path)?)
                .await
                .map_err(source_error)?,
        ),
        Command::ConfirmEditor { actual } => Some(
            service
                .confirm_editor_undo(original, undo_id, actual.checked()?)
                .await
                .map_err(error)?,
        ),
        Command::PrepareEditor { current } => match service
            .prepare_editor_undo(original, undo_id, current.checked()?)
            .await
            .map_err(error)?
        {
            UndoEditorAdmission::Existing(saved) => Some(saved),
            UndoEditorAdmission::Dispatch(edit) => {
                return Ok(Response {
                    schema_version: 1,
                    original_scope: scope,
                    undo_id,
                    result: Some(super::Response::Edit {
                        schema_version: 1,
                        scope: undo_scope,
                        expected: Observation {
                            path: edit.expected.path,
                            session_id: edit.expected.version.session_id,
                            revision: edit.expected.version.revision,
                            text: edit.expected.text,
                        },
                        text: edit.text,
                    }),
                });
            }
        },
    };
    Ok(Response {
        schema_version: 1,
        original_scope: scope,
        undo_id,
        result: saved.map(|saved| project(undo_scope, saved.application)),
    })
}

#[tauri::command]
pub(crate) async fn agent_writing_proposal_undo(
    store: tauri::State<'_, crate::storage::AppStore>,
    request: Request,
) -> Result<Response, Error> {
    #[cfg(target_os = "macos")]
    {
        execute(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn find(
    store: &crate::storage::AppStore,
    scope: Scope,
) -> Result<Option<Response>, Error> {
    use super::native::{error, project};
    use crate::agent::writing_submission::WritingSubmissionService;
    let saved = WritingSubmissionService::from_app_store(store)
        .find_latest_proposal_undo(scope.checked()?)
        .await
        .map_err(error)?;
    Ok(saved.map(|saved| {
        let undo_id = saved.application.operation_id;
        Response {
            schema_version: 1,
            undo_id,
            result: Some(project(
                Scope {
                    operation_id: undo_id,
                    ..scope.clone()
                },
                saved.application,
            )),
            original_scope: scope,
        }
    }))
}

#[tauri::command]
pub(crate) async fn agent_find_writing_proposal_undo(
    store: tauri::State<'_, crate::storage::AppStore>,
    scope: Scope,
) -> Result<Option<Response>, Error> {
    #[cfg(target_os = "macos")]
    {
        find(&store, scope).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, scope);
        Err(Error::UnsupportedPlatform)
    }
}
