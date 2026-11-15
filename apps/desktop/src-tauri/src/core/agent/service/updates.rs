//! Native long polls deliver accepted records without forcing SQL projection.
use super::*;
use std::time::Duration;
use tokio::time::{Instant, timeout_at};

impl AgentSessionService {
    pub(crate) async fn wait_for_accepted_history(
        &self,
        id: ConversationId,
        after_seq: u64,
        wait_ms: u32,
    ) -> Result<AcceptedHistoryUpdate, AgentServiceError> {
        if wait_ms > 30_000 || after_seq > 9_007_199_254_740_991 {
            return Err(StoreError::InvalidPage.into());
        }
        let _permit = self
            .update_waiters
            .try_acquire()
            .map_err(|_| AgentServiceError::Capacity)?;
        let notices = self.subscribe();
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms));
        self.wait_accepted_inner(id, after_seq, deadline, notices)
            .await
    }
    async fn wait_accepted_inner(
        &self,
        id: ConversationId,
        after_seq: u64,
        deadline: Instant,
        mut notices: broadcast::Receiver<CommitNotice>,
    ) -> Result<AcceptedHistoryUpdate, AgentServiceError> {
        loop {
            if let Some(update) = self
                .accepted_history
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .read(id, after_seq)?
            {
                return Ok(update);
            }
            if Instant::now() >= deadline {
                return Ok(AcceptedHistoryUpdate::idle());
            }
            loop {
                match timeout_at(deadline, notices.recv()).await {
                    Ok(Ok(notice)) if notice.conversation_id == id => break,
                    Ok(Ok(_)) => {
                        if Instant::now() >= deadline {
                            return Ok(AcceptedHistoryUpdate::idle());
                        }
                    }
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => break,
                    Ok(Err(broadcast::error::RecvError::Closed)) => {
                        return Err(AgentServiceError::Unavailable);
                    }
                    Err(_) => return Ok(AcceptedHistoryUpdate::idle()),
                }
            }
        }
    }
    /// Internal compatibility for watermark callers. The native command uses
    /// accepted records; an unknown/evicted legacy cursor resyncs SQL once.
    pub async fn wait_for_history(
        &self,
        id: ConversationId,
        after_seq: u64,
        wait_ms: u32,
    ) -> Result<Option<u64>, AgentServiceError> {
        if wait_ms > 30_000 || after_seq > 9_007_199_254_740_991 {
            return Err(StoreError::InvalidPage.into());
        }
        let _permit = self
            .update_waiters
            .try_acquire()
            .map_err(|_| AgentServiceError::Capacity)?;
        let notices = self.subscribe();
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms));
        let known = self
            .accepted_history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .watermark(id)
            .is_some();
        if !known {
            let page = self.read_since(id, after_seq, 1).await?;
            self.observe_history_snapshot(id, page.durable_count);
        }
        let update = self
            .wait_accepted_inner(id, after_seq, deadline, notices)
            .await?;
        if update.resync_required {
            let page = self.read_since(id, after_seq, 1).await?;
            self.observe_history_snapshot(id, page.durable_count);
            return Ok((page.durable_count > after_seq).then_some(page.durable_count));
        }
        Ok(update.watermark)
    }
}

#[cfg(test)]
#[path = "updates/tests.rs"]
mod tests;
