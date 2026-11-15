//! Original Core composition keeps context/evidence owners; pure candidate DTOs
//! are named forwards to their single shared Native SDK definitions.
pub(crate) use crate::ai::paper_analysis_contracts::PaperTemplateRef;
pub use research_workbench_native_sdk::candidate_contracts::{
    AiOverviewProfileContext, AiReadingProfileRef, PaperTemplateSnapshot,
};
pub(crate) use research_workbench_native_sdk::candidate_contracts::{
    CandidatePayload, OVERVIEW_FIELDS, PaperOverviewField, PaperOverviewPayload,
    template_field_identity, valid_template_snapshot,
};

pub use research_workbench_native_sdk::generation::AiMatrixSynthesisContext;

pub use research_workbench_native_sdk::candidate_review::contracts::{
    AiEvidenceBundle, AiEvidenceItem,
};
