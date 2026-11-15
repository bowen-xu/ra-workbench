//! Shared wire edits and queue state; resource authorization belongs to host adapters.
use super::repository::input_queue::Error;
use research_agent_core::input_queue::{InputQueue, QueueEdit, QueueError, QueuedInput};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[path = "queue_contract/storage.rs"]
mod storage;
pub use storage::QueueStorageError;
pub(crate) use storage::failure;
#[path = "queue_contract/status.rs"]
mod status;
pub use status::EditStatusResponse;
pub(crate) use status::edit_status;
#[path = "queue_contract/operation.rs"]
mod operation;
pub(crate) use operation::{execute_edit, read_edit_status, worker_failure};

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Edit {
    Restore {
        input_id: Uuid,
        source_input_id: Uuid,
        index: usize,
        text: String,
        #[serde(default)]
        user_input: Vec<research_agent_core::user_input::UserInput>,
    },
    Add {
        input_id: Uuid,
        text: String,
        #[serde(default)]
        user_input: Vec<research_agent_core::user_input::UserInput>,
    },
    Update {
        input_id: Uuid,
        text: String,
        #[serde(default)]
        user_input: Vec<research_agent_core::user_input::UserInput>,
    },
    Remove {
        input_id: Uuid,
    },
    Reorder {
        input_ids: Vec<Uuid>,
    },
    Pause {},
    Resume {},
    PauseInput {
        input_id: Uuid,
    },
    ResumeInput {
        input_id: Uuid,
    },
}
impl TryFrom<Edit> for QueueEdit {
    type Error = CommandError;
    fn try_from(edit: Edit) -> Result<Self, Self::Error> {
        Ok(match edit {
            Edit::Restore {
                input_id,
                source_input_id,
                index,
                text,
                user_input,
            } => Self::Restore {
                input: QueuedInput {
                    id: input_id,
                    text,
                    user_input,
                    origin: None,
                },
                source_id: source_input_id,
                index,
                paused: false,
            },
            Edit::Add {
                input_id,
                text,
                user_input,
            } => Self::Add(QueuedInput {
                user_input,
                origin: None,
                id: input_id,
                text,
            }),
            Edit::Update {
                input_id,
                text,
                user_input,
            } => Self::Update {
                id: input_id,
                text,
                user_input,
            },
            Edit::Remove { input_id } => Self::Remove(input_id),
            Edit::Reorder { input_ids } => {
                if input_ids.len() > 64 {
                    return Err(CommandError::Invalid);
                }
                Self::Reorder(input_ids)
            }
            Edit::Pause {} => Self::Pause,
            Edit::Resume {} => Self::Resume,
            Edit::PauseInput { input_id } => Self::PauseInput(input_id),
            Edit::ResumeInput { input_id } => Self::ResumeInput(input_id),
        })
    }
}
#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum CommandError {
    #[serde(rename = "QUEUE_STORAGE_FAILURE")]
    Storage(QueueStorageError),
    Invalid,
    Conflict,
    Busy,
    Paused,
    Capacity,
    Closed,
    Corrupt,
    Unavailable,
}
impl From<Error> for CommandError {
    fn from(error: Error) -> Self {
        match error {
            Error::Queue(QueueError::Invalid) => Self::Invalid,
            Error::Queue(QueueError::Conflict) => Self::Conflict,
            Error::Queue(QueueError::Busy) => Self::Busy,
            Error::Queue(QueueError::Paused) => Self::Paused,
            Error::Queue(QueueError::Capacity) => Self::Capacity,
            Error::Closed => Self::Closed,
            Error::Corrupt => Self::Corrupt,
            Error::Storage => Self::Unavailable,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    schema_version: u8,
    conversation_id: Uuid,
    revision: u64,
    paused: bool,
    preparing_input_id: Option<Uuid>,
    inputs: Vec<Input>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    input_id: Uuid,
    #[serde(skip_serializing_if = "is_false")]
    paused: bool,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    user_input: Vec<research_agent_core::user_input::UserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<research_agent_core::conversation::ConversationBinding>,
}
pub(crate) fn state(queue: InputQueue) -> Result<QueueState, CommandError> {
    if queue.revision() > 9_007_199_254_740_991 {
        return Err(CommandError::Corrupt);
    }
    Ok(QueueState {
        schema_version: 1,
        conversation_id: queue.conversation_id(),
        revision: queue.revision(),
        paused: queue.paused(),
        preparing_input_id: queue.preparing(),
        inputs: queue
            .inputs()
            .iter()
            .map(|input| Input {
                paused: queue.input_paused(input.id),
                user_input: input.user_input.clone(),
                input_id: input.id,
                text: input.text.clone(),
                origin: input.origin.clone(),
            })
            .collect(),
    })
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Identity of an explicitly confirmed queue preview, separate from user edits.
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Confirmation {
    pub input_id: Uuid,
    pub expected_revision: u64,
}
