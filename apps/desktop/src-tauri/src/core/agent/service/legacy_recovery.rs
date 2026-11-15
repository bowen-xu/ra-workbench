//! An unfinished legacy import is not a previously running model task.
use super::{
    AgentServiceError, AgentSessionService, ConversationId, DurableConversation, StoreError,
};

impl AgentSessionService {
    pub(super) async fn check_legacy_recovery(
        &self,
        id: ConversationId,
        session: &DurableConversation,
    ) -> Result<(), AgentServiceError> {
        // A partial import may stop at a completed turn, so an idle projection
        // is not evidence that this conversation is safe for a normal writer.
        // Empty reservations still use the legacy import's existing open path.
        if session.projection().seq() == 0 {
            return Ok(());
        }
        self.ensure_no_legacy_import(id).await
    }

    pub(super) async fn ensure_no_legacy_import(
        &self,
        id: ConversationId,
    ) -> Result<(), AgentServiceError> {
        let Some(database) = self.compaction_db.clone() else {
            // Only the in-memory/test constructor omits the application database.
            return Ok(());
        };
        let conversation_id = *id.as_uuid();
        let pending = tokio::task::spawn_blocking(move || {
            let db = database.lock().map_err(|_| AgentServiceError::Storage(StoreError::Persistence {
                diagnostic: format!("legacy_recovery_admission_failed conversation_id={conversation_id}"),
            }))?;
            db.query_row(
                "select exists(select 1 from agent_legacy_import_recovery_guards where conversation_id=?1)",
                [conversation_id.to_string()],
                |row| row.get::<_, bool>(0),
            ).map_err(|error| AgentServiceError::Storage(StoreError::Persistence {
                diagnostic: format!("legacy_recovery_query_failed conversation_id={conversation_id} sqlite_code={:?}", error.sqlite_error_code()),
            }))
        })
        .await
        .map_err(|_| AgentServiceError::Storage(StoreError::Persistence {
            diagnostic: format!("legacy_recovery_worker_failed conversation_id={conversation_id}"),
        }))??;
        if pending {
            return Err(AgentServiceError::LegacyMigrationIncomplete(
                conversation_id,
            ));
        }
        Ok(())
    }
}
