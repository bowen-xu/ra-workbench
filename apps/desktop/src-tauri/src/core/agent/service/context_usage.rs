//! Measurements share the original session's SQLite connection and executor.
use super::*;
use crate::agent::{context_usage::*, repository::context_usage as repository};
use research_agent_core::protocol::ModelIdentity;

fn map(error: repository::Error, id: Uuid) -> AgentServiceError {
    AgentServiceError::Storage(match error {
        repository::Error::Invalid => StoreError::InvalidBatch,
        repository::Error::NotFound => StoreError::NotFound {
            conversation_id: ConversationId::from_uuid(id),
        },
        repository::Error::Conflict => StoreError::WriterConflict,
        repository::Error::Corrupt => StoreError::CorruptHistory,
        repository::Error::Storage(_) => StoreError::Unavailable,
    })
}

impl AgentSessionService {
    pub(crate) async fn record_context_usage(
        &self,
        conversation: Uuid,
        sample: WindowUsageSample,
    ) -> Result<(), AgentServiceError> {
        let Some(db) = self.compaction_db.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| AgentServiceError::Unavailable)?;
            repository::record(&mut db, conversation, &sample)
                .map_err(|error| map(error, conversation))
        })
        .await
        .map_err(|_| AgentServiceError::Unavailable)?
    }

    pub async fn context_usage(
        &self,
        conversation: Uuid,
        model: ModelIdentity,
    ) -> Result<WindowUsageResponse, AgentServiceError> {
        let db = self
            .compaction_db
            .clone()
            .ok_or(AgentServiceError::Unavailable)?;
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| AgentServiceError::Unavailable)?;
            repository::read(&mut db, conversation, model).map_err(|error| map(error, conversation))
        })
        .await
        .map_err(|_| AgentServiceError::Unavailable)?
    }
}
