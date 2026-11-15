//! The session owner commits the exact source-bearing item before returning
//! model input. Journal admission independently verifies the audit record.
use super::{RecordedStopRun, StopResolution};
use crate::agent::service::{AgentServiceError, AgentSessionService};
use research_agent_core::{
    conversation::ConversationId,
    model::{ContentPart, ModelMessage},
    protocol::{EventPayload, HookPromptFragment, ItemKind},
    submission::SubmissionRecord,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StopPromptCommitError {
    #[error("stop hook invocation {0} requires the managed data interface")]
    Unmanaged(Uuid),
    #[error("stop hook invocation {0} has no continuation feedback")]
    NoFollowUp(Uuid),
    #[error("stop hook feedback commit failed (invocation {invocation_id}, item {item_id})")]
    Commit {
        invocation_id: Uuid,
        item_id: Uuid,
        #[source]
        source: AgentServiceError,
    },
}

/// Only a committed journal item authorizes the returned model message.
#[allow(
    dead_code,
    reason = "the task-owned Stop adapter will call this bridge"
)]
pub(crate) async fn persist_follow_up(
    service: &AgentSessionService,
    record: &SubmissionRecord,
    result: &RecordedStopRun,
) -> Result<ModelMessage, StopPromptCommitError> {
    if !service.has_logged_data() {
        return Err(StopPromptCommitError::Unmanaged(result.invocation_id));
    }
    let StopResolution::FollowUp { parts } = result.resolution() else {
        return Err(StopPromptCommitError::NoFollowUp(result.invocation_id));
    };
    let item_id = Uuid::new_v4();
    let fragments: Vec<_> = parts
        .into_iter()
        .map(|part| HookPromptFragment {
            run_id: part.run_id,
            input_text: part.input_text,
        })
        .collect();
    let message = ModelMessage::User {
        source_item_id: Some(item_id),
        content: fragments
            .iter()
            .map(|part| ContentPart::Text(part.input_text.clone()))
            .collect(),
    };
    service
        .commit_payload(
            ConversationId::from_uuid(record.conversation_id),
            record.request_id,
            EventPayload::ItemAccepted {
                turn_id: record.turn_id,
                item_id,
                kind: ItemKind::HookPrompt {
                    invocation_id: result.invocation_id,
                    fragments,
                },
                text: String::new(),
                phase: None,
                source: None,
            },
        )
        .await
        .map_err(|source| StopPromptCommitError::Commit {
            invocation_id: result.invocation_id,
            item_id,
            source,
        })?;
    Ok(message)
}
