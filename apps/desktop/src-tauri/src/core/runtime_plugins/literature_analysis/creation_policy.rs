//! The two original conversation decisions, replayed only against matching facts.
use super::error::AnalysisError;
use crate::ai::structured_resource_validation::{DecisionOrigin, Failure, FailureCause, Flow};
use research_workbench_native_sdk::analysis_workflow::{AnalysisWorkflowError, wire::*};
use uuid::Uuid;

pub(crate) enum CreationNeed {
    Availability(Box<WorkflowInputV1>),
    Conversation(Box<WorkflowInputV1>),
    Changed,
}
impl CreationNeed {
    pub(crate) fn input(&self) -> Option<&WorkflowInputV1> {
        match self {
            Self::Availability(input) | Self::Conversation(input) => Some(input),
            Self::Changed => None,
        }
    }
}
struct Availability {
    input: WorkflowInputV1,
    rejection: Option<AnalysisWorkflowError>,
    origin: DecisionOrigin,
}
struct Conversation {
    input: WorkflowInputV1,
    decision: ConversationDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(crate) struct CreationPolicy {
    availability: Option<Availability>,
    conversation: Option<Conversation>,
}
impl CreationPolicy {
    pub(crate) fn latest_origin(&self) -> Option<&DecisionOrigin> {
        self.conversation
            .as_ref()
            .map(|value| &value.origin)
            .or_else(|| self.availability.as_ref().map(|value| &value.origin))
    }
    pub(crate) fn resolve(
        &mut self,
        need: CreationNeed,
        decision: WorkflowDecisionV1,
        origin: DecisionOrigin,
    ) -> bool {
        match (need, decision) {
            (CreationNeed::Availability(input), WorkflowDecisionV1::Validated { rejection }) => {
                self.availability = Some(Availability {
                    input: *input,
                    rejection,
                    origin,
                });
                true
            }
            (CreationNeed::Conversation(input), WorkflowDecisionV1::Conversation { decision }) => {
                self.conversation = Some(Conversation {
                    input: *input,
                    decision,
                    origin,
                });
                true
            }
            _ => false,
        }
    }
    pub(crate) fn availability(
        &self,
        portrait_active: bool,
        analysis_active: bool,
    ) -> Result<(), Flow<AnalysisError, CreationNeed>> {
        let input = WorkflowInputV1::ValidateGeneration {
            portrait_active,
            analysis_active,
        };
        let Some(cached) = self.availability.as_ref() else {
            return Err(Flow::Need(CreationNeed::Availability(Box::new(input))));
        };
        if cached.input != input {
            return Err(Flow::Need(CreationNeed::Changed));
        }
        match &cached.rejection {
            None => Ok(()),
            Some(error) => Err(rejected(error.clone(), cached.origin.clone())),
        }
    }
    pub(crate) fn conversation(
        &self,
        unarchived: Option<String>,
    ) -> Result<Option<Uuid>, Flow<AnalysisError, CreationNeed>> {
        let input = WorkflowInputV1::SelectConversation { unarchived };
        let Some(cached) = self.conversation.as_ref() else {
            return Err(Flow::Need(CreationNeed::Conversation(Box::new(input))));
        };
        if cached.input != input {
            return Err(Flow::Need(CreationNeed::Changed));
        }
        match &cached.decision {
            ConversationDecisionV1::Reuse { conversation_id } => Ok(Some(*conversation_id)),
            ConversationDecisionV1::Create {} => Ok(None),
            ConversationDecisionV1::Rejected { error } => {
                Err(rejected(error.clone(), cached.origin.clone()))
            }
        }
    }
}
fn rejected(
    error: AnalysisWorkflowError,
    origin: DecisionOrigin,
) -> Flow<AnalysisError, CreationNeed> {
    Flow::Fail(Failure {
        error: error.into(),
        cause: FailureCause::NativeRejection(Box::new(origin)),
    })
}
