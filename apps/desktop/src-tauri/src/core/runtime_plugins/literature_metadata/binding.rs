//! Check the complete owned input echo without reimplementing domain preparation.
use super::{MetadataFailure, MetadataPhaseOutcome, MetadataProblem};
use research_workbench_native_sdk::metadata::*;

pub(crate) fn check(
    input: &MetadataInputV1,
    prepared: &MetadataPreparationV1,
) -> Result<(), MetadataFailure> {
    let echo = match prepared {
        MetadataPreparationV1::Update { plan } => MetadataInputV1::Update {
            input: plan.input.clone(),
        },
        MetadataPreparationV1::Confirm { outcome } => MetadataInputV1::Confirm {
            input: match outcome {
                MetadataConfirmOutcomeV1::Ready { input, .. }
                | MetadataConfirmOutcomeV1::Rejected { input, .. } => input.clone(),
            },
        },
        MetadataPreparationV1::FullApply { plan } => MetadataInputV1::FullApply {
            input: plan.input.clone(),
        },
        MetadataPreparationV1::FieldsApply { outcome } => MetadataInputV1::FieldsApply {
            input: match outcome.as_ref() {
                MetadataFieldsApplyOutcomeV1::Ready { input, .. }
                | MetadataFieldsApplyOutcomeV1::Rejected { input, .. } => input.clone(),
            },
        },
        MetadataPreparationV1::Reject { plan } => MetadataInputV1::Reject {
            input: plan.input.clone(),
        },
    };
    if *input == echo {
        Ok(())
    } else {
        Err(MetadataFailure::new(
            "preparation",
            MetadataProblem::Binding,
            MetadataPhaseOutcome::NotStarted,
        ))
    }
}
