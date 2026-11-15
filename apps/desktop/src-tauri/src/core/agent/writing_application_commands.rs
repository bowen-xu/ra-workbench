//! Typed application coordination for the writing host, never a model tool.
use super::writing_workspace_commands::{Buffer, Error};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[path = "writing_application_commands/discovery.rs"]
pub(crate) mod discovery;
#[cfg(target_os = "macos")]
#[path = "writing_application_commands/pending_save.rs"]
mod pending_save;
#[path = "writing_application_commands/restoration.rs"]
pub(crate) mod restoration;
#[path = "writing_application_commands/undo.rs"]
pub(crate) mod undo;
#[path = "writing_application_commands/undo_restoration.rs"]
pub(crate) mod undo_restoration;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub proposal_id: Uuid,
    pub operation_id: Uuid,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub path: String,
    pub session_id: Uuid,
    pub revision: Uuid,
    pub text: String,
}

#[cfg(target_os = "macos")]
pub(crate) fn checkpoint_source_error(
    cause: super::writing_submission::workspace::Error,
    scope: Option<super::repository::proposal_applications::ApplicationScope>,
) -> Error {
    match scope {
        Some(scope) => super::writing_workspace_commands::source_error(
            cause,
            scope.conversation,
            scope.manuscript,
            scope.operation,
            native::error,
        ),
        None => native::error(cause),
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Request {
    Begin {
        scope: Scope,
        buffers: Vec<Buffer>,
    },
    /// Resume an interrupted operation only after native source/editor
    /// preflight. The operation identity is existing; no new journal is made.
    Resume {
        scope: Scope,
        buffers: Vec<Buffer>,
    },
    Read {
        scope: Scope,
    },
    PrepareEditor {
        scope: Scope,
        current: Observation,
    },
    ConfirmEditor {
        scope: Scope,
        actual: Observation,
    },
    DeferEditor {
        scope: Scope,
        path: String,
    },
    /// Host owns save/navigation exclusion; a live editor target must never be
    /// dispatched through this action. The journal independently checks target.
    ApplyDisk {
        scope: Scope,
        path: String,
    },
    ReconcileDisk {
        scope: Scope,
        path: String,
    },
    RollbackDisk {
        scope: Scope,
    },
    #[serde(rename_all = "camelCase")]
    PrepareRollback {
        scope: Scope,
        buffers: Vec<Buffer>,
        #[serde(default)]
        checkpoint_versions:
            Vec<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
    #[serde(rename_all = "camelCase")]
    RollbackRemainder {
        scope: Scope,
        #[serde(default)]
        buffers: Option<Vec<Buffer>>,
        #[serde(default)]
        checkpoint_versions:
            Vec<crate::agent::writing_submission::workspace::UndoCheckpointVersion>,
    },
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Target {
    Disk,
    #[serde(rename_all = "camelCase")]
    Editor {
        session_id: Uuid,
        revision: Uuid,
        result_revision: Option<Uuid>,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    path: String,
    phase: &'static str,
    target: Target,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Response {
    #[serde(rename_all = "camelCase")]
    State {
        schema_version: u8,
        scope: Scope,
        status: &'static str,
        files: Vec<File>,
    },
    #[serde(rename_all = "camelCase")]
    Edit {
        schema_version: u8,
        scope: Scope,
        expected: Observation,
        text: String,
    },
}

#[cfg(target_os = "macos")]
#[path = "writing_application_commands/native.rs"]
mod native;
#[cfg(target_os = "macos")]
pub(crate) use native::{execute, find};

#[tauri::command]
pub(crate) async fn agent_find_writing_proposal_application(
    store: tauri::State<'_, crate::storage::AppStore>,
    request: super::writing_proposal_commands::ProposalRequest,
) -> Result<Option<Response>, Error> {
    #[cfg(target_os = "macos")]
    {
        find(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[tauri::command]
pub(crate) async fn agent_writing_proposal_application(
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
