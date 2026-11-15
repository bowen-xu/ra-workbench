//! Separate restore protocol: an original apply response never authorizes compensation.
use super::*;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Read {
        scope: Scope,
        path: String,
    },
    #[serde(rename_all = "camelCase")]
    Prepare {
        scope: Scope,
        restore_id: Uuid,
        current: Observation,
        #[serde(default)]
        checkpoint_version:
            Option<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
    #[serde(rename_all = "camelCase")]
    Confirm {
        scope: Scope,
        restore_id: Uuid,
        actual: Observation,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restoration {
    id: Uuid,
    session_id: Uuid,
    expected_revision: Uuid,
    result_revision: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Response {
    #[serde(rename_all = "camelCase")]
    State {
        schema_version: u8,
        application: super::Response,
        path: String,
        restoration: Option<Restoration>,
    },
    #[serde(rename_all = "camelCase")]
    Edit {
        schema_version: u8,
        scope: Scope,
        restore_id: Uuid,
        expected: Observation,
        text: String,
    },
}
#[cfg(target_os = "macos")]
pub(crate) async fn execute(
    store: &crate::storage::AppStore,
    request: Request,
) -> Result<Response, Error> {
    execute_for(store, request, None).await
}

#[cfg(target_os = "macos")]
pub(super) async fn execute_for(
    store: &crate::storage::AppStore,
    request: Request,
    original: Option<crate::agent::repository::proposal_applications::ApplicationScope>,
) -> Result<Response, Error> {
    use super::native::{error, project};
    use crate::agent::writing_submission::{
        WritingSubmissionService, workspace::EditorRestoreAdmission,
    };
    let scope = match &request {
        Request::Read { scope, .. }
        | Request::Prepare { scope, .. }
        | Request::Confirm { scope, .. } => scope.clone(),
    };
    let checked = scope.checked()?;
    if let Some(origin) = original
        && (origin.conversation != checked.conversation
            || origin.manuscript != checked.manuscript
            || origin.proposal != checked.proposal
            || origin.operation == checked.operation)
    {
        return Err(Error::InvalidInput);
    }
    let service = WritingSubmissionService::from_app_store(store);
    let path = match request {
        Request::Read { path, .. } => {
            research_agent_core::workspace::validate_path(&path)
                .map_err(|_| Error::InvalidInput)?;
            path
        }
        Request::Prepare {
            restore_id,
            current,
            checkpoint_version,
            ..
        } => {
            if restore_id.is_nil()
                || restore_id == checked.operation
                || original.is_some_and(|o| restore_id == o.operation)
            {
                return Err(Error::InvalidInput);
            }
            let current = current.checked()?;
            let path = current.path.clone();
            let admission = match original {
                Some(origin) => {
                    service
                        .prepare_undo_editor_restoration_with_checkpoint(
                            origin,
                            checked.operation,
                            restore_id,
                            current,
                            checkpoint_version,
                        )
                        .await
                }
                None => {
                    service
                        .prepare_editor_restoration_with_checkpoint(
                            checked,
                            restore_id,
                            current,
                            checkpoint_version,
                        )
                        .await
                }
            }
            .map_err(|cause| {
                crate::agent::writing_workspace_commands::source_error(
                    cause,
                    checked.conversation,
                    checked.manuscript,
                    checked.operation,
                    error,
                )
            })?;
            match admission {
                EditorRestoreAdmission::Dispatch(edit) => {
                    return Ok(Response::Edit {
                        schema_version: 1,
                        scope,
                        restore_id: edit.operation,
                        expected: Observation {
                            path: edit.expected.path,
                            session_id: edit.expected.version.session_id,
                            revision: edit.expected.version.revision,
                            text: edit.expected.text,
                        },
                        text: edit.text,
                    });
                }
                EditorRestoreAdmission::Existing { .. } => path,
            }
        }
        Request::Confirm {
            restore_id, actual, ..
        } => {
            if restore_id.is_nil()
                || restore_id == checked.operation
                || original.is_some_and(|o| restore_id == o.operation)
            {
                return Err(Error::InvalidInput);
            }
            let actual = actual.checked()?;
            let path = actual.path.clone();
            match original {
                Some(origin) => {
                    service
                        .confirm_undo_editor_restoration(
                            origin,
                            checked.operation,
                            restore_id,
                            actual,
                        )
                        .await
                        .map_err(error)?;
                }
                None => {
                    service
                        .confirm_editor_restoration(checked, restore_id, actual)
                        .await
                        .map_err(error)?;
                }
            }
            path
        }
    };
    let (application, restoration) = match original {
        Some(origin) => {
            let (saved, restoration) = service
                .read_undo_editor_restoration(origin, checked.operation, path.clone())
                .await
                .map_err(error)?;
            (saved.application, restoration)
        }
        None => service
            .read_editor_restoration(checked, path.clone())
            .await
            .map_err(error)?,
    };
    Ok(Response::State {
        schema_version: 1,
        application: project(scope, application),
        path,
        restoration: restoration.map(|r| Restoration {
            id: r.id,
            session_id: r.expected.session_id,
            expected_revision: r.expected.revision,
            result_revision: r.result_revision,
        }),
    })
}
#[tauri::command]
pub(crate) async fn agent_writing_editor_restoration(
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
