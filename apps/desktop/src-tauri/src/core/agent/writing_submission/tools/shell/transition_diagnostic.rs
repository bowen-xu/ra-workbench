//! Retain only the original native successor identity, never file contents.
use super::*;
use research_agent_core::runner::ToolOutcomeDiagnostic;

impl Host {
    pub(super) fn transition_error(&self, cause: &'static str) -> RunError {
        let mut detail = json!({"code":"WRITING_SUCCESSOR_UNCONFIRMED", "phase":"successor-preparation", "safeCause":cause,
            "conversationId":self.record.conversation_id, "requestId":self.record.request_id,
            "turnId":self.record.turn_id, "contextSnapshotId":self.record.context_snapshot_id,
            "manuscriptId":self.binding.manuscript_id});
        if let Ok(request) = self.transition_request.read()
            && let Some(request) = request.as_ref()
        {
            detail["operationId"] = json!(request.request_id);
            detail["leaseId"] = json!(request.lease_id);
            detail["resultId"] = json!(request.predecessor_delivery_id);
        }
        ToolOutcomeDiagnostic::try_from_value(detail)
            .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
            .unwrap_or(RunError::ToolOutcomeUnknown)
    }
}
