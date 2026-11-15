//! Keep journal admission and replacement installation under one session owner.
use super::*;
use crate::agent::cancellation::AgentCancellation;
use research_agent_core::context::{
    checkpoint::ContextCheckpoint,
    checkpoint_store::{CheckpointInstallation, CheckpointStoreError},
};
impl AgentSessionService {
    pub(crate) async fn new_context_request(
        &self,
        conversation: Uuid,
        turn: Uuid,
        consume: bool,
    ) -> Result<bool, AgentServiceError> {
        let slot = self.slot(ConversationId::from_uuid(conversation), false)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        if consume {
            Ok(session.take_new_context_window_request(turn)?)
        } else {
            session.request_new_context_window(turn)?;
            Ok(false)
        }
    }
    pub(crate) async fn take_compact_session_start(
        &self,
        conversation: Uuid,
    ) -> Result<Option<ContextCheckpoint>, CheckpointStoreError> {
        use research_agent_core::context::checkpoint_store::ContextCheckpointStore;
        let Some(db) = self.compaction_db.clone() else {
            return Ok(None);
        };
        super::super::repository::context_window_store::SqliteContextCheckpointStore::new(db)
            .take_pending_session_start(conversation)
            .await
    }
    pub(crate) async fn install_context_checkpoint(
        &self,
        checkpoint: ContextCheckpoint,
        expected_revision: u64,
        cancellation: AgentCancellation,
    ) -> Result<CheckpointInstallation, CheckpointStoreError> {
        let id = ConversationId::from_uuid(checkpoint.conversation_id);
        let slot = self
            .slot(id, false)
            .map_err(|_| CheckpointStoreError::Inactive)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(CheckpointStoreError::Inactive)?;
        if session.needs_recovery()
            || session.projection().active_turn() != Some(checkpoint.turn_id)
        {
            return Err(CheckpointStoreError::Inactive);
        }
        if session.projection().seq() != expected_revision {
            return Err(CheckpointStoreError::Conflict);
        }
        let db = self
            .compaction_db
            .clone()
            .ok_or(CheckpointStoreError::Unavailable)?;
        let identity = if session.has_token_budget_window()
            || checkpoint.binding.kind
                == research_agent_core::context::compaction_strategy::CompactionKind::ResetWindow
        {
            Some(
                self.restore_token_budget_window_owned(
                    session,
                    checkpoint.conversation_id,
                    checkpoint.turn_id,
                )
                .await?,
            )
        } else {
            None
        };
        if identity.is_some_and(|identity| identity.first == checkpoint.window_id) {
            return Err(CheckpointStoreError::Invalid);
        }
        if let Some(identity) = identity
            && identity.current != checkpoint.window_id
            && identity.previous != Some(checkpoint.window_id)
        {
            // Prove advancement before the durable commit; installation cannot
            // leave a committed window followed by a predictable state error.
            let mut candidate =
                research_agent_core::context::token_budget::TokenBudgetWindow::restore(
                    identity.number,
                    identity.first,
                    identity.previous,
                    identity.current,
                )
                .map_err(|_| CheckpointStoreError::Invalid)?;
            candidate
                .advance(checkpoint.window_id)
                .map_err(|_| CheckpointStoreError::Invalid)?;
        }
        let turn = checkpoint.turn_id;
        let next = checkpoint.window_id;
        // Do not select/drop this durable operation. The storage executor checks
        // cancellation at its commit gate and completes the winning transaction.
        let installed =
            super::super::repository::context_window_store::SqliteContextCheckpointStore::new(db)
                .install_owned(checkpoint, expected_revision, Some(cancellation))
                .await?;
        if installed == CheckpointInstallation::Installed {
            if let Some(identity) = identity {
                session
                    .advance_token_budget_window(turn, identity.current, next)
                    .map_err(|_| CheckpointStoreError::Invalid)?;
            }
            session.clear_new_context_window_request();
        }
        Ok(installed)
    }
}
