//! Hook prompt rendering adapted from Codex protocol/src/items.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0, Copyright 2025 OpenAI).
//! The task owner must journal these fragments before sampling or exposing them.
use crate::agent::repository::stop_hook_audit::{Outcome, Status};
use research_agent_core::runner::{StopHandlerOutput, StopHandlerStatus, aggregate_stop_outputs};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopPromptPart {
    pub run_id: Uuid,
    /// One model input-text part, with the upstream hook_prompt XML wrapper.
    pub input_text: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopResolution {
    Canceled,
    Stop { reason: Option<String> },
    Continue,
    FollowUp { parts: Vec<StopPromptPart> },
}
impl StopResolution {
    /// Incomplete admissions remain unresolved. Before consuming a recovered
    /// result, the caller must authorize the conversation and verify the turn
    /// and journal boundary are still visible in its current history.
    pub fn from_recorded(
        record: &crate::agent::repository::stop_hook_audit::Recorded,
    ) -> Option<Self> {
        record
            .finished
            .as_ref()
            .map(|finished| resolution(finished.canceled, &finished.outcomes))
    }
}

#[derive(Serialize)]
#[serde(rename = "hook_prompt")]
struct HookPromptXml<'a> {
    #[serde(rename = "@hook_run_id")]
    run_id: &'a str,
    #[serde(rename = "$text")]
    text: &'a str,
}

pub(super) fn resolution(canceled: bool, outcomes: &[(Uuid, Uuid, Outcome)]) -> StopResolution {
    if canceled {
        return StopResolution::Canceled;
    }
    let completed: Vec<_> = outcomes
        .iter()
        .filter_map(|(_, run, outcome)| match outcome {
            Outcome::Completed {
                status,
                system_message,
            } => {
                let status = match status {
                    Status::Completed => StopHandlerStatus::Completed,
                    Status::Failed { diagnostic } => StopHandlerStatus::Failed {
                        diagnostic: diagnostic.clone(),
                    },
                    Status::Stopped { reason } => StopHandlerStatus::Stopped {
                        reason: reason.clone(),
                    },
                    Status::Blocked { feedback } => StopHandlerStatus::Blocked {
                        feedback: feedback.clone(),
                    },
                };
                Some((
                    run.to_string(),
                    StopHandlerOutput {
                        status,
                        system_message: system_message.clone(),
                    },
                ))
            }
            Outcome::Canceled | Outcome::NotStarted => None,
        })
        .collect();
    let aggregate =
        aggregate_stop_outputs(completed.iter().map(|(run, output)| (run.as_str(), output)));
    if aggregate.stopped {
        return StopResolution::Stop {
            reason: aggregate.stop_reason,
        };
    }
    let parts: Vec<_> = aggregate
        .fragments
        .into_iter()
        .filter_map(|fragment| {
            let run_id = Uuid::parse_str(&fragment.hook_run_id).ok()?;
            let input_text = quick_xml::se::to_string(&HookPromptXml {
                run_id: &fragment.hook_run_id,
                text: &fragment.text,
            })
            .ok()?;
            Some(StopPromptPart { run_id, input_text })
        })
        .collect();
    if parts.is_empty() {
        StopResolution::Continue
    } else {
        StopResolution::FollowUp { parts }
    }
}

#[cfg(test)]
#[path = "prompt/tests.rs"]
mod tests;
