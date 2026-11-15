//! Drain application-owned tasks before releasing durable session writers.
use super::*;

impl AgentSessionService {
    /// Closing admission is permanent. On storage failure keep the application
    /// alive and allow another shutdown call to retry from authoritative history.
    pub async fn shutdown(&self) -> Result<(), AgentServiceError> {
        #[cfg(unix)]
        self.mcp_runtime.write().await.take();
        #[cfg(unix)]
        self.processes
            .begin_shutdown()
            .map_err(|_| AgentServiceError::Unavailable)?;
        self.tasks
            .shutdown()
            .await
            .map_err(|_| AgentServiceError::Unavailable)?;
        #[cfg(unix)]
        self.processes
            .shutdown()
            .await
            .map_err(|_| AgentServiceError::Unavailable)?;
        let slots = self
            .sessions
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?
            .iter()
            .map(|(id, slot)| (*id, slot.clone()))
            .collect::<Vec<_>>();
        for (id, slot) in slots {
            let mut owner = slot.lock().await;
            if owner
                .as_ref()
                .is_none_or(DurableConversation::needs_recovery)
            {
                owner.take();
                match DurableConversation::resume(self.store.clone(), fresh_lease(id)).await {
                    Ok(session) => *owner = Some(session),
                    // An initialization that never persisted has nothing to flush.
                    Err(SessionError::Storage(StoreError::NotFound { .. })) => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if let Some(session) = owner.as_mut() {
                // All owned tasks have joined. Never replay external effects.
                match self.check_legacy_recovery(id, session).await {
                    Err(AgentServiceError::LegacyMigrationIncomplete(_)) => {
                        owner.take();
                        continue;
                    }
                    Err(error) => return Err(error),
                    Ok(()) => {}
                }
                if let Some(receipt) = session
                    .interrupt_recovered_turn(Uuid::new_v4(), now_ms()?)
                    .await?
                {
                    self.notify(id, &receipt);
                }
            }
            // Every acknowledged event is already durable; there is no deferred
            // UI buffer to flush. Release only after reconciliation succeeds.
            owner.take();
        }
        self.invalidate_all_accepted_history();
        self.invalidate_history_snapshots(None);
        Ok(())
    }
}

#[cfg(test)]
#[path = "shutdown/tests.rs"]
mod tests;
