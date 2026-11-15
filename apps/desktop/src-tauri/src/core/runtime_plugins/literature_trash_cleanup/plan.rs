use super::{TrashCleanupFailure, TrashCleanupOutcome, TrashCleanupProblem};
use research_workbench_native_sdk::trash_cleanup::{
    CleanupPlanRejectionCodeV1, CleanupPlanV1, TrashCleanupPreparationV1,
};
use sha2::{Digest, Sha256};

const PLAN_SHA256: &str = "fb0d5090e40cecd2d317d7a13a1e9216ddc27c4303089a38f49748576612bbf4";

/// Construction is restricted to the complete, versioned Native policy binding.
/// A cycle remains deferred until the original per-item cleanup position.
pub(crate) struct CheckedCleanupPlan(Preparation);
enum Preparation {
    Ready(CleanupPlanV1),
    PlanCycle,
}
impl CheckedCleanupPlan {
    pub(super) fn ready(&self) -> Result<&CleanupPlanV1, TrashCleanupProblem> {
        match &self.0 {
            Preparation::Ready(plan) => Ok(plan),
            Preparation::PlanCycle => Err(TrashCleanupProblem::PlanCycle),
        }
    }
}

pub(crate) fn check_preparation(
    outcome: TrashCleanupPreparationV1,
) -> Result<CheckedCleanupPlan, TrashCleanupFailure> {
    match outcome {
        TrashCleanupPreparationV1::Rejected {
            code: CleanupPlanRejectionCodeV1::PlanCycle,
        } => Ok(CheckedCleanupPlan(Preparation::PlanCycle)),
        TrashCleanupPreparationV1::Ready { plan } => {
            let matches = serde_json::to_vec(&plan)
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)) == PLAN_SHA256)
                .unwrap_or(false);
            if matches {
                Ok(CheckedCleanupPlan(Preparation::Ready(plan)))
            } else {
                Err(TrashCleanupFailure::new(
                    "preparation",
                    TrashCleanupProblem::Binding,
                    TrashCleanupOutcome::NotStarted,
                ))
            }
        }
    }
}
