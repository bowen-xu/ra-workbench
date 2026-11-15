//! Original confirmation workflows over Core storage and finite selected literature policy.
mod overview;
mod paper;
mod repository;
mod resources;
mod settlement;
mod undo;
use crate::ai::error::AiError;
pub(crate) use crate::ai::structured_resource_validation::Flow;
pub(crate) use repository::ConfirmationRepository;
use research_workbench_native_sdk::paper_confirmation::{
    PaperConfirmationRequestV1,
    contracts::*,
    wire::{PaperConfirmationDecisionV1, PaperConfirmationInputV1},
};
pub(crate) use resources::ConfirmationResources;

pub(crate) trait ConfirmationPolicy {
    type Pause;
    fn decide(
        &mut self,
        input: PaperConfirmationInputV1,
    ) -> Result<PaperConfirmationDecisionV1, Flow<AiError, Self::Pause>>;
    fn explicit_settlement(&self) -> bool {
        true
    }
    fn complete_module(&mut self) {}
}
#[derive(Default)]
pub(crate) struct ConfirmationState {
    confirm: Option<paper::ConfirmProgress>,
    undo: Option<paper::UndoProgress>,
}
pub(crate) enum ConfirmationResult {
    Overview(Box<AiOverviewConfirmationResult>),
    Paper(Box<AiPaperReviewConfirmationResult>),
}
impl<P: ConfirmationPolicy> ConfirmationRepository<'_, '_, P> {
    pub(crate) fn execute(
        &mut self,
        request: PaperConfirmationRequestV1,
        state: &mut ConfirmationState,
    ) -> Result<ConfirmationResult, Flow<AiError, P::Pause>> {
        match request {
            PaperConfirmationRequestV1::ConfirmOverview { draft } => overview::confirm(self, draft)
                .map(|value| ConfirmationResult::Overview(Box::new(value))),
            PaperConfirmationRequestV1::UndoOverview { draft } => {
                undo::undo(self, draft).map(|value| ConfirmationResult::Overview(Box::new(value)))
            }
            PaperConfirmationRequestV1::ConfirmPaper { draft } => {
                paper::confirm(self, draft, &mut state.confirm)
                    .map(|value| ConfirmationResult::Paper(Box::new(value)))
            }
            PaperConfirmationRequestV1::UndoPaper { draft } => {
                paper::undo(self, draft, &mut state.undo)
                    .map(|value| ConfirmationResult::Paper(Box::new(value)))
            }
        }
    }
}
pub(super) fn mismatch<P>() -> Flow<AiError, P> {
    Flow::original(AiError::TemporaryUnavailable)
}

pub(super) fn unit_policy<P: ConfirmationPolicy>(
    policy: &mut P,
    scope: &str,
    input: research_workbench_native_sdk::paper_confirmation::unit::UnitInputV1,
) -> Result<
    research_workbench_native_sdk::paper_confirmation::unit::UnitDecisionV1,
    Flow<AiError, P::Pause>,
> {
    match policy.decide(PaperConfirmationInputV1::Unit {
        scope_key: scope.into(),
        input,
    })? {
        PaperConfirmationDecisionV1::Unit { decision } => Ok(decision),
        _ => Err(mismatch()),
    }
}
