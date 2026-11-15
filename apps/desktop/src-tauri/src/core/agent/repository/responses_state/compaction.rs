//! Validate installed account/model state in the same replay transaction.
use super::*;
use research_agent_core::context::compaction_strategy::CompactionOutput;

pub(super) fn bind_installed(
    state: &ResponsesContinuation,
    db: &Connection,
    scope: &ResponsesStateScope,
    account: Uuid,
    policy: Option<crate::agent::execution_profile::NativeCompactionPolicy>,
) -> Result<(), ModelError> {
    let checkpoint = super::super::context_windows::read_active(db, scope.conversation_id)
        .map_err(|_| ModelError::InvalidStream)?;
    let Some(checkpoint) = checkpoint else {
        return Ok(());
    };
    if !matches!(checkpoint.output, CompactionOutput::TextSummary(_))
        && checkpoint.binding.model != scope.model
    {
        let target = policy.ok_or(ModelError::UnsupportedCapability)?;
        let target_binding = target.binding(scope.model.clone());
        if !target.accepts_remote_binding(&target_binding, &checkpoint.binding) {
            return Err(ModelError::UnsupportedCapability);
        }
        use super::super::context_policy::{ContextAuthentication, FrozenContextPolicy};
        let encoded: Option<String> = db
            .query_row(
                "select policy_json from agent_context_policies where turn_id=?1",
                [checkpoint.turn_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| ModelError::Unavailable)?;
        let producer: FrozenContextPolicy =
            serde_json::from_str(&encoded.ok_or(ModelError::UnsupportedCapability)?)
                .map_err(|_| ModelError::InvalidStream)?;
        producer.validate()?;
        if producer.authentication != ContextAuthentication::CodexBackend
            || producer.binding != checkpoint.binding
        {
            return Err(ModelError::UnsupportedCapability);
        }
    }
    match &checkpoint.output {
        // The transaction has already selected a fresh continuation when the
        // active window ID changed. Do not clear responses from this new epoch.
        CompactionOutput::ResetWindow => Ok(()),
        CompactionOutput::TextSummary(_) => Ok(()),
        CompactionOutput::RemoteOpaque(item) => {
            if checkpoint.credential_scope_id != Some(account) {
                return Err(ModelError::Authentication);
            }
            let retained_inputs =
                crate::agent::model::responses_transport::retained_v2_inputs(&checkpoint)?;
            state.bind_compaction_sync(account, &scope.model, item, retained_inputs)
        }
        CompactionOutput::RemoteWindow(items) => {
            if checkpoint.credential_scope_id != Some(account) {
                return Err(ModelError::Authentication);
            }
            state.bind_window_with_canonical_sync(
                account,
                &scope.model,
                items,
                if checkpoint.schema_version == 4 {
                    checkpoint
                        .canonical_context
                        .as_ref()
                        .map_or(0, |context| context.messages.len())
                } else {
                    0
                },
            )
        }
    }
}
