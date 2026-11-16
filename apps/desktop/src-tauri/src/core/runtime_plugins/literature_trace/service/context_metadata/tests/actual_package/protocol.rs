//! Test oracles consume the selected package's public finite wire contracts.
//! Process admission, framing, result reads and shutdown use the original Core transport.
use research_workbench_native_sdk::{
    analysis_workflow::{candidate::*, wire::*},
    candidate_contracts::{CandidatePayload, CandidateValidationError},
    catalog::{CatalogInput, CatalogOutput},
    paper_context_wire::{decode_payload, encode_payload},
};

pub(super) fn catalog(input: CatalogInput) -> CatalogOutput {
    let (catalog, key) = super::actual_package();
    crate::runtime_plugins::native::package::tests::catalog_oracle(&catalog, &key, input)
}

pub(super) fn workflow(input: WorkflowInputV1) -> WorkflowDecisionV1 {
    let request = WorkflowWireInputV1 {
        cbor_base64: encode_payload(&input).unwrap(),
    };
    let CatalogOutput::AnalysisWorkflow { outcome } =
        catalog(CatalogInput::DecideAnalysisWorkflow {
            input: Box::new(request.clone()),
        })
    else {
        panic!("workflow operation")
    };
    assert_eq!(outcome.input, request);
    let decision = decode_payload(&outcome.decision_cbor_base64).unwrap();
    assert!(input.matches_decision(&decision));
    decision
}

pub(super) fn candidate(input: CandidateInputV1) -> CandidateDecisionV1 {
    let WorkflowDecisionV1::Candidate { decision } = workflow(WorkflowInputV1::Candidate {
        input: Box::new(input),
    }) else {
        panic!("candidate operation")
    };
    decision
}

pub(super) fn decode_candidate_value(
    value: serde_json::Value,
    keys: &std::collections::HashSet<String>,
) -> Result<CandidatePayload, CandidateValidationError> {
    decode_candidate_content(&value.to_string(), keys)
}

pub(super) fn decode_candidate_content(
    content: &str,
    keys: &std::collections::HashSet<String>,
) -> Result<CandidatePayload, CandidateValidationError> {
    match candidate(CandidateInputV1::Decode {
        content: content.to_owned(),
        evidence_keys: keys.iter().cloned().collect(),
    }) {
        CandidateDecisionV1::Decoded {
            payload_cbor_base64,
        } => Ok(decode_payload(&payload_cbor_base64).unwrap()),
        CandidateDecisionV1::Rejected { error } => Err(error),
        _ => panic!("candidate decode result"),
    }
}

pub(super) fn validate_candidate(input: CandidateInputV1) -> Result<(), CandidateValidationError> {
    match candidate(input) {
        CandidateDecisionV1::Validated {} | CandidateDecisionV1::Prepared { .. } => Ok(()),
        CandidateDecisionV1::Rejected { error } => Err(error),
        _ => panic!("candidate validation result"),
    }
}

pub(super) fn review(
    input: research_workbench_native_sdk::candidate_review::wire::ReviewInputV1,
) -> research_workbench_native_sdk::candidate_review::wire::ReviewDecisionV1 {
    let WorkflowDecisionV1::CandidateReview { decision } =
        workflow(WorkflowInputV1::CandidateReview {
            input: Box::new(input),
        })
    else {
        panic!("review operation")
    };
    decision
}

pub(super) fn item_review(
    input: research_workbench_native_sdk::candidate_review::items::ItemReviewInputV1,
) -> research_workbench_native_sdk::candidate_review::items::ItemReviewDecisionV1 {
    let WorkflowDecisionV1::SuggestionReview { decision } =
        workflow(WorkflowInputV1::SuggestionReview {
            input: Box::new(input),
        })
    else {
        panic!("item review operation")
    };
    decision
}

pub(super) fn paper_review(
    input: research_workbench_native_sdk::candidate_review::paper::PaperReviewInputV1,
) -> research_workbench_native_sdk::candidate_review::paper::PaperReviewDecisionV1 {
    let WorkflowDecisionV1::PaperConceptReview { decision } =
        workflow(WorkflowInputV1::PaperConceptReview {
            input: Box::new(input),
        })
    else {
        panic!("paper review operation")
    };
    decision
}

pub(super) fn template(
    input: &research_workbench_native_sdk::analysis_workflow::template::TemplateInputV1,
) -> Result<
    Option<research_workbench_native_sdk::candidate_contracts::AiOverviewProfileContext>,
    research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
> {
    use research_workbench_native_sdk::analysis_workflow::template::TemplateDecisionV1;
    let WorkflowDecisionV1::Template { decision } = workflow(WorkflowInputV1::Template {
        input: Box::new(input.clone()),
    }) else {
        panic!("template operation")
    };
    match decision {
        TemplateDecisionV1::Verified {} => Ok(None),
        TemplateDecisionV1::Resolved { profile } => Ok(Some(profile)),
        TemplateDecisionV1::Rejected { error } => Err(error),
    }
}

pub(super) fn build_context(
    input: &research_workbench_native_sdk::paper_context::PaperContextBuildInputV1,
) -> research_workbench_native_sdk::paper_context::PreparedPaperContextV1 {
    use research_workbench_native_sdk::paper_context_wire::*;
    let CatalogOutput::PaperContext { outcome } = catalog(CatalogInput::PreparePaperContext {
        input: Box::new(PaperContextInputV1::Build {
            cbor_base64: encode_payload(input).unwrap(),
        }),
    }) else {
        panic!("paper context operation")
    };
    let PaperContextDecisionV1::Built {
        prepared_cbor_base64,
    } = outcome.decision
    else {
        panic!("built context")
    };
    decode_payload(&prepared_cbor_base64).unwrap()
}

pub(super) fn metadata(
    input: &research_workbench_native_sdk::context_metadata::ContextMetadataInputV1,
) -> Result<String, research_workbench_native_sdk::context_metadata::ContextMetadataRejectionV1> {
    use research_workbench_native_sdk::context_metadata::*;
    let CatalogOutput::ContextMetadata { outcome } =
        catalog(CatalogInput::ProjectContextMetadata {
            input: Box::new(ContextMetadataRequestV1 {
                cbor_base64: encode_payload(input).unwrap(),
            }),
        })
    else {
        panic!("metadata operation")
    };
    match outcome.decision {
        ContextMetadataDecisionV1::Projected { body_json } => Ok(body_json),
        ContextMetadataDecisionV1::Rejected { reason } => Err(reason),
    }
}

pub(super) fn overview_task(
    input: &research_workbench_native_sdk::analysis_workflow::overview_task::OverviewTaskInputV1,
) -> research_workbench_native_sdk::generation::AiTaskSpecDraft {
    let WorkflowDecisionV1::OverviewTask { draft_cbor_base64 } =
        workflow(WorkflowInputV1::OverviewTask {
            input: input.clone(),
        })
    else {
        panic!("overview task operation")
    };
    decode_payload(&draft_cbor_base64).unwrap()
}

pub(super) fn image_set(requested: &[String], available: &[String]) -> Result<(), String> {
    match workflow(WorkflowInputV1::ValidateDoclingImages {
        requested_ids: requested.into(),
        available_ids: available.into(),
    }) {
        WorkflowDecisionV1::Validated { rejection } => {
            rejection.map_or(Ok(()), |error| Err(error.to_string()))
        }
        _ => panic!("image set operation"),
    }
}

pub(super) fn context_prepare(
    input: research_workbench_native_sdk::paper_context_prepare::PaperPreparationInputV1,
) -> research_workbench_native_sdk::paper_context_prepare::PaperPreparationDecisionV1 {
    use research_workbench_native_sdk::paper_context_wire::*;
    let CatalogOutput::PaperContext { outcome } = catalog(CatalogInput::PreparePaperContext {
        input: Box::new(PaperContextInputV1::Prepare {
            cbor_base64: encode_payload(&input).unwrap(),
        }),
    }) else {
        panic!("context preparation operation")
    };
    let PaperContextDecisionV1::Prepared {
        decision_cbor_base64,
    } = outcome.decision
    else {
        panic!("context preparation result")
    };
    decode_payload(&decision_cbor_base64).unwrap()
}

pub(super) fn docling_row(
    run: &str,
    item: &research_workbench_native_sdk::paper_context::PaperAiReadingContextItem,
) -> Result<
    Option<research_workbench_native_sdk::analysis_workflow::docling::DoclingEvidenceRow>,
    research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
> {
    use research_workbench_native_sdk::analysis_workflow::{AnalysisWorkflowError, docling::*};
    let input = DoclingRowInputV1 {
        run_id: run.into(),
        item: item.clone(),
    };
    let WorkflowDecisionV1::DoclingRow { decision } = workflow(WorkflowInputV1::DoclingRow {
        input_cbor_base64: encode_payload(&input).unwrap(),
    }) else {
        panic!("Docling row operation")
    };
    match decision {
        DoclingRowDecisionV1::Ready { row } => Ok(Some(row)),
        DoclingRowDecisionV1::Skipped {} => Ok(None),
        DoclingRowDecisionV1::Unavailable {} => Err(AnalysisWorkflowError::TemporaryUnavailable),
    }
}

pub(super) fn docling_prompt(
    input: &research_workbench_native_sdk::analysis_workflow::docling::DoclingPromptInputV1,
) -> Result<
    research_workbench_native_sdk::generation::provider::ProviderInput,
    research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
> {
    let WorkflowDecisionV1::DoclingPrompt { result_cbor_base64 } =
        workflow(WorkflowInputV1::DoclingPrompt {
            input_cbor_base64: encode_payload(input).unwrap(),
        })
    else {
        panic!("Docling prompt operation")
    };
    decode_payload(&result_cbor_base64).unwrap()
}

pub(super) fn render_prompt(
    input: &research_workbench_native_sdk::analysis_workflow::prompt::TaskPromptInputV1,
) -> Result<
    research_workbench_native_sdk::analysis_workflow::prompt::PaperTaskPrompt,
    research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
> {
    let WorkflowDecisionV1::Prompt { result_cbor_base64 } =
        workflow(WorkflowInputV1::RenderPrompt {
            input_cbor_base64: encode_payload(input).unwrap(),
        })
    else {
        panic!("prompt operation")
    };
    decode_payload(&result_cbor_base64).unwrap()
}

pub(super) fn validate_reuse(
    regenerate: bool,
    template: Option<&str>,
    existing: Option<
        &research_workbench_native_sdk::analysis_history::contracts::PaperAnalysisWorkflow,
    >,
) -> Result<(), research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError> {
    let WorkflowDecisionV1::Validated { rejection } = workflow(WorkflowInputV1::ValidateReuse {
        regenerate,
        template_json: template.map(str::to_owned),
        existing: existing.cloned().map(Box::new),
    }) else {
        panic!("reuse operation")
    };
    rejection.map_or(Ok(()), Err)
}

pub(super) fn history(
    input: research_workbench_native_sdk::analysis_history::HistoryInputV1,
) -> research_workbench_native_sdk::analysis_history::HistoryResultV1 {
    use research_workbench_native_sdk::analysis_history::HistoryWireInputV1;
    let CatalogOutput::AnalysisHistory { outcome } =
        catalog(CatalogInput::ProjectAnalysisHistory {
            input: Box::new(HistoryWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            }),
        })
    else {
        panic!("history operation")
    };
    decode_payload(&outcome.result_cbor_base64).unwrap()
}

pub(super) fn request_hash(
    draft: &research_workbench_native_sdk::candidate_review::items::AiSuggestionItemDecisionDraft,
) -> Result<String, CandidateValidationError> {
    use research_workbench_native_sdk::candidate_review::items::*;
    let ItemReviewDecisionV1::Draft { result } = item_review(ItemReviewInputV1::Draft {
        draft: draft.clone(),
    }) else {
        panic!("draft hash operation")
    };
    result
}

pub(super) fn generic_prepare(
    input: &research_workbench_native_sdk::candidate_review::generic::GenericPromotionInputV1,
) -> Result<
    research_workbench_native_sdk::candidate_review::generic::GenericPromotionPlanV1,
    CandidateValidationError,
> {
    use research_workbench_native_sdk::candidate_review::wire::*;
    let ReviewDecisionV1::GenericPromotion { result } = review(ReviewInputV1::GenericPromotion {
        input: Box::new(input.clone()),
    }) else {
        panic!("generic promotion operation")
    };
    result.map(|plan| *plan)
}

pub(super) fn confirmation(
    input: research_workbench_native_sdk::paper_confirmation::wire::PaperConfirmationInputV1,
) -> research_workbench_native_sdk::paper_confirmation::wire::PaperConfirmationResultV1 {
    use research_workbench_native_sdk::paper_confirmation::wire::*;
    let CatalogOutput::PaperConfirmation { outcome } =
        catalog(CatalogInput::PreparePaperConfirmation {
            input: Box::new(PaperConfirmationWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            }),
        })
    else {
        panic!("confirmation operation")
    };
    decode_payload(&outcome.result_cbor_base64).unwrap()
}

pub(super) fn qualify(
    candidate: &research_workbench_native_sdk::candidate_review::contracts::AiCandidate,
    draft: &research_workbench_native_sdk::paper_confirmation::contracts::AiOverviewFieldConfirmationDraft,
) -> Result<
    research_workbench_native_sdk::paper_confirmation::overview::QualifiedOverviewV1,
    CandidateValidationError,
> {
    use research_workbench_native_sdk::paper_confirmation::wire::*;
    match confirmation(PaperConfirmationInputV1::OverviewCandidate {
        stored_review_state: candidate.review_state.clone(),
        candidate: Box::new(candidate.clone()),
        draft: draft.clone(),
    })? {
        PaperConfirmationDecisionV1::OverviewCandidate { qualified } => Ok(*qualified),
        _ => panic!("overview qualification operation"),
    }
}

pub(super) fn prepared(input: CandidateInputV1) -> Result<String, CandidateValidationError> {
    match candidate(input) {
        CandidateDecisionV1::Prepared { canonical_json } => Ok(canonical_json),
        CandidateDecisionV1::Rejected { error } => Err(error),
        _ => panic!("prepared candidate result"),
    }
}
