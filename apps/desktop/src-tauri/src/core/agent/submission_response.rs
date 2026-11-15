//! Shared native admission response. Dropping this response never stops a turn.
use super::service::SubmissionAdmission;
use research_agent_core::protocol::TurnOutcome;
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    pub already_started: bool,
    pub outcome: Option<TurnOutcome>,
}

impl StartResponse {
    /// Attach only an unambiguous retained owner for the already-proven turn.
    pub(crate) fn with_retained_owner(
        mut self,
        tasks: &super::tasks::AgentTaskRegistry,
    ) -> Result<Self, super::tasks::TaskRegistryError> {
        if self.already_started && self.run_id.is_none() {
            self.run_id = tasks
                .find_turn_receipt(
                    research_agent_core::conversation::ConversationId::from_uuid(
                        self.conversation_id,
                    ),
                    self.turn_id,
                )?
                .map(|receipt| receipt.run_id());
        }
        Ok(self)
    }

    pub(crate) fn from_admission(
        conversation_id: Uuid,
        request_id: Uuid,
        admitted: SubmissionAdmission,
    ) -> Self {
        let (turn_id, run_id, already_started, outcome) = match admitted {
            SubmissionAdmission::Started(owned) => {
                (owned.turn_id, Some(owned.task.run_id()), false, None)
            }
            SubmissionAdmission::AlreadyStarted { turn_id, outcome } => {
                (turn_id, None, true, outcome)
            }
        };
        Self {
            schema_version: 1,
            conversation_id,
            request_id,
            turn_id,
            run_id,
            already_started,
            outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{service::OwnedTurn, tasks::AgentTaskRegistry};
    use research_agent_core::conversation::ConversationId;

    #[tokio::test]
    async fn replay_exposes_only_the_unambiguous_original_run_identity() {
        let registry = AgentTaskRegistry::default();
        let conversation = Uuid::new_v4();
        let request = Uuid::new_v4();
        let turn = Uuid::new_v4();
        let task = registry
            .start_with_cleanup_claim(
                ConversationId::from_uuid(conversation),
                Some(turn),
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                |_| async { Ok(()) },
                |result| async move { result },
            )
            .unwrap();
        let run = task.run_id();
        let started = StartResponse::from_admission(
            conversation,
            request,
            SubmissionAdmission::Started(OwnedTurn {
                turn_id: turn,
                task,
            }),
        );
        assert_eq!(
            serde_json::to_value(started).unwrap()["runId"],
            run.to_string()
        );
        let replay = StartResponse::from_admission(
            conversation,
            request,
            SubmissionAdmission::AlreadyStarted {
                turn_id: turn,
                outcome: None,
            },
        );
        let replay = replay.with_retained_owner(&registry).unwrap();
        assert_eq!(replay.run_id, Some(run));
        let other = StartResponse::from_admission(
            Uuid::new_v4(),
            request,
            SubmissionAdmission::AlreadyStarted {
                turn_id: turn,
                outcome: None,
            },
        )
        .with_retained_owner(&registry)
        .unwrap();
        assert!(other.run_id.is_none());
    }
}
