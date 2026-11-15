use super::*;
use crate::agent::repository::{
    context_window_store::SqliteContextCheckpointStore, context_windows::identity::WindowIdentity,
};
use research_agent_core::context::checkpoint_store::CheckpointStoreError;

impl AgentSessionService {
    pub(crate) async fn initial_window_metadata(
        &self,
        conversation: Uuid,
        turn: Uuid,
        signal: &crate::agent::cancellation::AgentCancellation,
    ) -> Result<String, CheckpointStoreError> {
        let identity = self.prepare_token_budget_window(conversation, turn).await?;
        if identity.number != 0 {
            return Err(CheckpointStoreError::Conflict);
        }
        let db = self
            .compaction_db
            .clone()
            .ok_or(CheckpointStoreError::Unavailable)?;
        let store = SqliteContextCheckpointStore::new(db);
        let history = self
            .open(ConversationId::from_uuid(conversation))
            .await
            .map_err(|_| CheckpointStoreError::Unavailable)?;
        if let Some(saved) = store
            .initial_metadata(conversation, identity.current, history.seq())
            .await?
        {
            return Ok(saved);
        }
        // Never hold the session lock across external memory I/O: steering can
        // still enter its existing queue. Recheck ownership before publication.
        let hint = tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(CheckpointStoreError::Canceled),
            hint = self.thread_hint(conversation) => hint,
        };
        let slot = self
            .slot(ConversationId::from_uuid(conversation), false)
            .map_err(|_| CheckpointStoreError::Inactive)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(CheckpointStoreError::Inactive)?;
        let current = self
            .restore_token_budget_window_owned(session, conversation, turn)
            .await?;
        if current != identity {
            return Err(CheckpointStoreError::Conflict);
        }
        let research_agent_core::model::ModelMessage::Developer(body) =
            (research_agent_core::context::token_budget::TokenBudgetContext {
                conversation_id: conversation,
                first_window_id: identity.first,
                previous_window_id: None,
                window_id: identity.current,
                thread_hint: hint,
            })
            .into_message()
        else {
            return Err(CheckpointStoreError::Invalid);
        };
        store
            .record_initial_metadata(
                crate::agent::repository::context_windows::initial_metadata::Snapshot {
                    conversation,
                    window: identity.current,
                    boundary: session.projection().seq(),
                    turn,
                    body,
                },
                signal.clone(),
            )
            .await
    }
    pub(crate) async fn thread_hint(&self, conversation: Uuid) -> Option<String> {
        #[cfg(unix)]
        {
            self.mcp_runtime().await?.thread_hint(conversation).await
        }
        #[cfg(not(unix))]
        {
            let _ = conversation;
            None
        }
    }
    pub(crate) async fn context_fragments(
        &self,
        conversation: Uuid,
        window: Option<Uuid>,
        after: u64,
    ) -> Result<HashMap<Uuid, Vec<String>>, research_agent_core::runner::RunError> {
        let Some(db) = self.compaction_db.clone() else {
            return Ok(HashMap::new());
        };
        let batches = SqliteContextCheckpointStore::new(db)
            .context_fragments(conversation, window, after)
            .await?;
        let mut fragments: HashMap<Uuid, Vec<String>> = HashMap::new();
        for batch in batches {
            fragments
                .entry(batch.anchor_item_id)
                .or_default()
                .extend(batch.fragments);
        }
        Ok(fragments)
    }
    pub(crate) async fn token_budget_decision(
        &self,
        conversation: Uuid,
        turn: Uuid,
        policy: &research_agent_core::context::token_budget::TokenBudgetPolicy,
        status: research_agent_core::context::token_budget::TokenBudgetStatus,
        needs_follow_up: bool,
    ) -> Result<research_agent_core::context::token_budget::TokenBudgetDecision, CheckpointStoreError>
    {
        let slot = self
            .slot(ConversationId::from_uuid(conversation), false)
            .map_err(|_| CheckpointStoreError::Inactive)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(CheckpointStoreError::Inactive)?;
        let identity = self
            .restore_token_budget_window_owned(session, conversation, turn)
            .await?;
        let prepared = session
            .prepare_post_sampling_token_budget(
                turn,
                identity.current,
                policy,
                status,
                needs_follow_up,
            )
            .map_err(|_| CheckpointStoreError::Conflict)?;
        let fragments: Vec<String> = prepared
            .decision()
            .reminder
            .iter()
            .chain(prepared.decision().fallback.iter())
            .cloned()
            .collect();
        if fragments.is_empty() {
            return session
                .commit_post_sampling_token_budget(turn, prepared)
                .map_err(|_| CheckpointStoreError::Conflict);
        }
        let history = session.projection();
        let projected = history.turn(turn).ok_or(CheckpointStoreError::Inactive)?;
        let anchor = projected
            .item_order
            .iter()
            .rev()
            .find(|id| {
                projected.items.get(id).is_some_and(|item| {
                    item.completed
                        && matches!(
                            item.kind,
                            research_agent_core::protocol::ItemKind::AssistantMessage
                                | research_agent_core::protocol::ItemKind::ToolResult { .. }
                        )
                })
            })
            .copied()
            .ok_or(CheckpointStoreError::Invalid)?;
        let batch = crate::agent::repository::context_fragments::FragmentBatch {
            conversation_id: conversation,
            window_id: identity.current,
            boundary_seq: history.seq(),
            turn_id: turn,
            anchor_item_id: anchor,
            fragments,
        };
        let db = self
            .compaction_db
            .clone()
            .ok_or(CheckpointStoreError::Unavailable)?;
        let mut persistence_error = None;
        let result = session
            .persist_post_sampling_token_budget(turn, prepared, async {
                match SqliteContextCheckpointStore::new(db)
                    .record_context_fragments(batch)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        persistence_error = Some(error);
                        Err(SessionError::Storage(
                            research_agent_core::conversation::StoreError::Unavailable,
                        ))
                    }
                }
            })
            .await;
        result.map_err(|_| persistence_error.unwrap_or(CheckpointStoreError::Unavailable))
    }
    pub(crate) async fn prepare_token_budget_window(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<WindowIdentity, CheckpointStoreError> {
        let slot = self
            .slot(ConversationId::from_uuid(conversation), false)
            .map_err(|_| CheckpointStoreError::Inactive)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(CheckpointStoreError::Inactive)?;
        self.restore_token_budget_window_owned(session, conversation, turn)
            .await
    }

    pub(super) async fn restore_token_budget_window_owned(
        &self,
        session: &mut DurableConversation,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<WindowIdentity, CheckpointStoreError> {
        if session.needs_recovery() || session.projection().active_turn() != Some(turn) {
            return Err(CheckpointStoreError::Inactive);
        }
        let db = self
            .compaction_db
            .clone()
            .ok_or(CheckpointStoreError::Unavailable)?;
        let identity = SqliteContextCheckpointStore::new(db)
            .window_identity(conversation)
            .await?;
        session
            .restore_token_budget_window(
                turn,
                identity.number,
                identity.first,
                identity.previous,
                identity.current,
            )
            .map_err(|_| CheckpointStoreError::Conflict)?;
        Ok(identity)
    }
}
