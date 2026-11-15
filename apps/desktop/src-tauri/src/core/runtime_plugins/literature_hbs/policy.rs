use super::HbsReadingEvidenceResources;
use crate::{
    ai::{
        candidate_contracts::AiEvidenceBundle,
        error::AiError,
        structured_resource_validation::{
            DecisionOrigin, Failure, FailureCause, Flow, StructuredPolicy, ValidationSources,
        },
    },
    hbs_storage::boundary::{HbsNeed, HbsSources},
};
use research_workbench_native_sdk::{
    candidate_contracts::{CandidateValidationError, EvidenceQuote},
    editable_template::EditableTemplateInputV1,
    review_selection::UnreviewedSelectionInputV1,
    structured_validation::{StructuredTargetV1, StructuredValidationInputV1},
};
use serde_json::Value;

struct CachedValidation {
    input: StructuredValidationInputV1,
    result: Result<Value, CandidateValidationError>,
    origin: DecisionOrigin,
}
enum CachedDecision {
    Validation(CachedValidation),
    EditableTemplate {
        input: EditableTemplateInputV1,
        result: Result<Value, String>,
        origin: DecisionOrigin,
    },
    Unreviewed {
        input: UnreviewedSelectionInputV1,
        selected: Vec<String>,
    },
}
#[derive(Default)]
pub(crate) struct PrefixPolicy {
    slots: Vec<CachedDecision>,
    cursor: usize,
}
impl PrefixPolicy {
    pub(crate) fn restart(&mut self) {
        self.cursor = 0;
    }
    pub(crate) fn store(
        &mut self,
        slot: usize,
        input: StructuredValidationInputV1,
        result: Result<Value, CandidateValidationError>,
        origin: DecisionOrigin,
    ) -> bool {
        if slot != self.slots.len() {
            return false;
        }
        self.slots
            .push(CachedDecision::Validation(CachedValidation {
                input,
                result,
                origin,
            }));
        true
    }
    pub(crate) fn store_selection(
        &mut self,
        slot: usize,
        input: UnreviewedSelectionInputV1,
        selected: Vec<String>,
    ) -> bool {
        if slot != self.slots.len() {
            return false;
        }
        self.slots
            .push(CachedDecision::Unreviewed { input, selected });
        true
    }
    pub(crate) fn store_template(
        &mut self,
        slot: usize,
        input: EditableTemplateInputV1,
        result: Result<Value, String>,
        origin: DecisionOrigin,
    ) -> bool {
        if slot != self.slots.len() {
            return false;
        }
        self.slots.push(CachedDecision::EditableTemplate {
            input,
            result,
            origin,
        });
        true
    }
}
impl StructuredPolicy for PrefixPolicy {
    type Pause = HbsNeed;
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<AiError, HbsNeed>> {
        let input = EditableTemplateInputV1 {
            candidate_json: value.to_string(),
        };
        let slot = self.cursor;
        self.cursor += 1;
        if let Some(CachedDecision::EditableTemplate {
            input: cached,
            result,
            origin,
        }) = self.slots.get(slot)
            && cached == &input
        {
            return result.clone().map_err(|message| {
                Flow::Fail(Failure {
                    error: AiError::InvalidInput(message),
                    cause: FailureCause::NativeRejection(Box::new(DecisionOrigin {
                        package_key: origin.package_key.clone(),
                        instance_id: origin.instance_id.clone(),
                        native_request_id: origin.native_request_id.clone(),
                    })),
                })
            });
        }
        self.slots.truncate(slot);
        Err(Flow::Need(HbsNeed::EditableTemplate { slot, input }))
    }
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, HbsNeed>> {
        let input = StructuredValidationInputV1 {
            target: if kind == "portrait" {
                StructuredTargetV1::Portrait
            } else {
                StructuredTargetV1::Overview
            },
            candidate_json: value.to_string(),
            evidence: evidence
                .items
                .iter()
                .map(|item| EvidenceQuote {
                    citation_key: item.citation_key.clone(),
                    quote_exact: item.quote_exact.clone(),
                })
                .collect(),
        };
        let slot = self.cursor;
        self.cursor += 1;
        if let Some(CachedDecision::Validation(cached)) = self.slots.get(slot)
            && cached.input == input
        {
            return cached.result.clone().map_err(|error| {
                Flow::Fail(Failure {
                    error: AiError::from(error),
                    cause: FailureCause::NativeRejection(Box::new(DecisionOrigin {
                        package_key: cached.origin.package_key.clone(),
                        instance_id: cached.origin.instance_id.clone(),
                        native_request_id: cached.origin.native_request_id.clone(),
                    })),
                })
            });
        }
        // A changed prefix invalidates all following slots. There is no LRU,
        // fixed slot cap, or reuse of a later equal but differently ordered call.
        self.slots.truncate(slot);
        Err(Flow::Need(HbsNeed::Structured { slot, input }))
    }
}
impl crate::structured_resources::ReviewPolicy for PrefixPolicy {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, HbsNeed> {
        let slot = self.cursor;
        self.cursor += 1;
        if let Some(CachedDecision::Unreviewed {
            input: cached,
            selected,
        }) = self.slots.get(slot)
            && cached == &input
        {
            return Ok(selected.clone());
        }
        self.slots.truncate(slot);
        Err(HbsNeed::Unreviewed { slot, input })
    }
}

#[derive(Default)]
pub(crate) struct RuntimeHbsSources {
    pub(crate) policy: PrefixPolicy,
    pub(crate) reading: Option<HbsReadingEvidenceResources>,
}
impl HbsSources for RuntimeHbsSources {
    type Policy = PrefixPolicy;
    fn policy_only(&mut self) -> Result<&mut PrefixPolicy, Flow<String, HbsNeed>> {
        Ok(&mut self.policy)
    }
    fn require(&mut self) -> Result<ValidationSources<'_, PrefixPolicy>, Flow<String, HbsNeed>> {
        let reading = self.reading.as_ref().ok_or(Flow::Need(HbsNeed::Sources))?;
        Ok(ValidationSources {
            policy: &mut self.policy,
            reading,
        })
    }
}

#[cfg(test)]
mod tests;
