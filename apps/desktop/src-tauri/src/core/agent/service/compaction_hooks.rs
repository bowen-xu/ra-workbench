//! Await audit commits inside the existing task, including cancellation completion.
use super::*;
use research_agent_core::context::compaction_hooks::{
    CompactionHookNotification, CompactionHookOutcome, CompactionHookRequest, CompactionHookResult,
};
impl AgentSessionService {
    pub(crate) async fn compact_hook_contexts(
        &self,
        conversation: Uuid,
    ) -> Result<
        Vec<super::super::repository::compaction_hook_audit::context::ContextBatch>,
        AgentServiceError,
    > {
        let Some(db) = self.compaction_db.clone() else {
            return Ok(vec![]);
        };
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| AgentServiceError::Unavailable)?;
            super::super::repository::compaction_hook_audit::context::read_current(
                &db,
                conversation,
            )
            .map_err(|_| AgentServiceError::Unavailable)
        })
        .await
        .map_err(|_| AgentServiceError::Unavailable)?
    }
    pub(crate) async fn record_compaction_hook_result(
        &self,
        request: CompactionHookRequest,
        result: CompactionHookResult,
        handlers: Vec<(Uuid, Option<CompactionHookOutcome>)>,
    ) -> Result<(), AgentServiceError> {
        let db = self
            .compaction_db
            .clone()
            .ok_or(AgentServiceError::Unavailable)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_millis()).ok())
            .ok_or(AgentServiceError::Unavailable)?;
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| AgentServiceError::Unavailable)?;
            super::super::repository::compaction_hook_audit::context::record_result_with_handlers(
                &mut db, &request, &result, now, &handlers,
            )
            .map_err(|_| AgentServiceError::Unavailable)
        })
        .await
        .map_err(|_| AgentServiceError::Unavailable)?
    }

    pub(crate) async fn record_compaction_hook(
        &self,
        request: CompactionHookRequest,
        notification: CompactionHookNotification,
        handlers: Vec<Uuid>,
    ) -> Result<(), AgentServiceError> {
        let db = self
            .compaction_db
            .clone()
            .ok_or(AgentServiceError::Unavailable)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_millis()).ok())
            .ok_or(AgentServiceError::Unavailable)?;
        // Do not select/drop storage work: success must be durable before running
        // the hook, and canceled hooks must finish recording their actual status.
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| AgentServiceError::Unavailable)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| AgentServiceError::Unavailable)?;
            super::super::repository::compaction_hook_audit::record(
                &tx,
                &request,
                notification,
                now,
            )
            .map_err(|_| AgentServiceError::Unavailable)?;
            let audit_seq = tx.last_insert_rowid();
            let handlers: Vec<_> = handlers.into_iter().map(|id| (id, None)).collect();
            super::super::repository::compaction_hook_audit::handlers::record(
                &tx, audit_seq, &handlers,
            )
            .map_err(|_| AgentServiceError::Unavailable)?;
            tx.commit().map_err(|_| AgentServiceError::Unavailable)
        })
        .await
        .map_err(|_| AgentServiceError::Unavailable)?
    }
}
