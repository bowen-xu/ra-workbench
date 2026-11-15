//! Typed candidate compatibility and explicit, application-selected policy calls.
pub(crate) use super::candidate_contracts::{
    CandidatePayload, OVERVIEW_FIELDS, PaperOverviewPayload,
};
use super::{error::AiError, generation_policy::GenerationPolicySource};
use std::collections::HashSet;

pub(crate) fn decode_candidate_value(
    policy: GenerationPolicySource,
    value: serde_json::Value,
    allowed: &HashSet<String>,
) -> Result<CandidatePayload, AiError> {
    (policy.decode_value)(value, allowed)
}
