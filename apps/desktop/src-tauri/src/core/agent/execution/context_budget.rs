//! Independent byte protection includes protocol windows outside normalized history.
use super::*;

impl AgentExecution {
    pub(super) fn context_input_limit(
        &self,
        checkpoint: Option<&research_agent_core::context::checkpoint::ContextCheckpoint>,
    ) -> Result<usize, RunError> {
        use research_agent_core::context::compaction_strategy::CompactionOutput;
        let Some(CompactionOutput::RemoteWindow(items)) = checkpoint.map(|window| &window.output)
        else {
            return Ok(self.max_input_bytes);
        };
        let bytes = serde_json::to_vec(items)
            .map_err(|_| ModelError::InvalidStream)?
            .len();
        self.max_input_bytes
            .checked_sub(bytes)
            .ok_or(RunError::Budget)
    }
}
