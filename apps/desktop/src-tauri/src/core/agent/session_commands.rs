//! Session admission and durable history IPC; no arbitrary event writes exposed.
use super::service::{AgentServiceError, AgentSessionService};
use research_agent_core::{
    conversation::{ConversationId, StoreError},
    protocol::AgentEvent,
    session::SessionError,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
const MAX_SAFE: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRequest {
    pub conversation_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryRequest {
    pub conversation_id: Uuid,
    pub after_seq: u64,
    pub limit: u16,
    #[serde(default)]
    pub snapshot_id: Option<Uuid>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub watermark: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub after_seq: u64,
    pub events: Vec<AgentEvent>,
    pub watermark: u64,
    pub next_after_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<Uuid>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionCommandError {
    InvalidInput,
    Cancelled,
    NotFound,
    Busy,
    RecoveryRequired,
    Corrupt,
    Unavailable,
    HistorySnapshotExpired,
    HistorySnapshotTooLarge,
    #[serde(untagged)]
    Persistence {
        code: &'static str,
        message: String,
    },
}
impl From<AgentServiceError> for SessionCommandError {
    fn from(error: AgentServiceError) -> Self {
        match error {
            AgentServiceError::Session(SessionError::RecoveryRequired) => Self::RecoveryRequired,
            AgentServiceError::Session(SessionError::Storage(error))
            | AgentServiceError::Storage(error) => match error {
                StoreError::NotFound { .. } => Self::NotFound,
                StoreError::InvalidPage | StoreError::InvalidBatch => Self::InvalidInput,
                StoreError::CorruptHistory => Self::Corrupt,
                StoreError::WriterConflict | StoreError::AlreadyExists => Self::Busy,
                StoreError::Unavailable => Self::Unavailable,
                StoreError::Persistence { diagnostic } => Self::Persistence {
                    code: "data_persistence_failed",
                    message: diagnostic,
                },
            },
            AgentServiceError::NotOpen => Self::NotFound,
            AgentServiceError::Active
            | AgentServiceError::Capacity
            | AgentServiceError::RollbackConflict => Self::Busy,
            AgentServiceError::Session(SessionError::InvalidBatch) => Self::InvalidInput,
            AgentServiceError::Session(SessionError::Projection(_)) => Self::Corrupt,
            AgentServiceError::LegacyMigrationIncomplete(id) => Self::Persistence {
                code: "agent_legacy_migration_incomplete",
                message: format!(
                    "旧阅读对话迁移尚未完成，原历史已保留，请核对迁移状态。会话 ID：{id}"
                ),
            },
            AgentServiceError::Unavailable => Self::Unavailable,
        }
    }
}
#[tauri::command]
pub async fn agent_create_conversation(
    service: State<'_, Arc<AgentSessionService>>,
    request: SessionRequest,
) -> Result<SessionResponse, SessionCommandError> {
    let projection = service
        .create_or_open(ConversationId::from_uuid(request.conversation_id))
        .await?;
    Ok(SessionResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        watermark: projection.seq(),
    })
}
#[tauri::command]
pub async fn agent_open_conversation(
    service: State<'_, Arc<AgentSessionService>>,
    request: SessionRequest,
) -> Result<SessionResponse, SessionCommandError> {
    let projection = service
        .open(ConversationId::from_uuid(request.conversation_id))
        .await?;
    Ok(SessionResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        watermark: projection.seq(),
    })
}
#[tauri::command]
pub async fn agent_read_history(
    service: State<'_, Arc<AgentSessionService>>,
    request: HistoryRequest,
) -> Result<HistoryResponse, SessionCommandError> {
    read_page(&service, request).await
}
pub(crate) async fn read_page(
    service: &AgentSessionService,
    request: HistoryRequest,
) -> Result<HistoryResponse, SessionCommandError> {
    if request.after_seq > MAX_SAFE || request.limit == 0 || request.limit > 500 {
        return Err(SessionCommandError::InvalidInput);
    }
    use super::service::history_snapshot::HistorySnapshotError;
    let id = ConversationId::from_uuid(request.conversation_id);
    let query =
        research_agent_core::conversation::HistoryQuery::new(id, request.after_seq, request.limit)
            .map_err(AgentServiceError::from)?;
    let frozen = service
        .read_frozen_history(query, request.snapshot_id)
        .await
        .map_err(|error| match error {
            HistorySnapshotError::Expired => SessionCommandError::HistorySnapshotExpired,
            HistorySnapshotError::TooLarge => SessionCommandError::HistorySnapshotTooLarge,
            HistorySnapshotError::InvalidCursor => SessionCommandError::InvalidInput,
            HistorySnapshotError::Storage(error) => {
                SessionCommandError::from(AgentServiceError::Storage(error))
            }
        })?;
    let (page, snapshot_id) = if let Some(frozen) = frozen {
        (frozen.page, Some(frozen.snapshot_id))
    } else {
        (
            service
                .read_since(id, request.after_seq, request.limit)
                .await?,
            None,
        )
    };
    if page.durable_count > MAX_SAFE {
        return Err(SessionCommandError::Corrupt);
    }
    if request.after_seq > page.durable_count {
        return Err(SessionCommandError::InvalidInput);
    }
    let last_seq = request.after_seq + page.items.len() as u64;
    if last_seq > page.durable_count
        || page.items.iter().enumerate().any(|(index, event)| {
            event.conversation_id().as_uuid() != &request.conversation_id
                || event.seq() != request.after_seq + index as u64 + 1
        })
        || page.next_offset != (last_seq < page.durable_count).then_some(last_seq)
        || (last_seq < page.durable_count && page.items.is_empty())
    {
        return Err(SessionCommandError::Corrupt);
    }
    service.observe_history_snapshot(
        ConversationId::from_uuid(request.conversation_id),
        page.durable_count,
    );
    Ok(HistoryResponse {
        snapshot_id,
        schema_version: 1,
        conversation_id: request.conversation_id,
        after_seq: request.after_seq,
        events: page.items,
        watermark: page.durable_count,
        next_after_seq: page.next_offset,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitRequest {
    pub conversation_id: Uuid,
    pub after_seq: u64,
    pub wait_ms: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub after_seq: u64,
    pub watermark: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<AgentEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resync_required: Option<bool>,
}
#[tauri::command]
pub async fn agent_wait_for_history(
    service: State<'_, Arc<AgentSessionService>>,
    request: WaitRequest,
) -> Result<WaitResponse, SessionCommandError> {
    wait_for_history(&service, request).await
}

pub(super) async fn wait_for_history(
    service: &AgentSessionService,
    request: WaitRequest,
) -> Result<WaitResponse, SessionCommandError> {
    let update = service
        .wait_for_accepted_history(
            ConversationId::from_uuid(request.conversation_id),
            request.after_seq,
            request.wait_ms,
        )
        .await?;
    Ok(WaitResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        after_seq: request.after_seq,
        watermark: update.watermark,
        events: Some(update.events),
        resync_required: Some(update.resync_required),
    })
}

#[cfg(test)]
#[path = "session_commands/tests.rs"]
mod tests;
