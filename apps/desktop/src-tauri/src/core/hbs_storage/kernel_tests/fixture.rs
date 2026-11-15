use super::*;
use crate::ai::candidate_contracts::AiEvidenceBundle;
use research_workbench_native_sdk::structured_validation::{
    StructuredTargetV1, StructuredValidationInputV1,
};

pub(super) struct Policy {
    pub calls: usize,
    pub pause_at: Option<usize>,
    pub reject_at: Option<usize>,
    pub panic: bool,
}
impl StructuredPolicy for Policy {
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<crate::ai::error::AiError, Self::Pause>> {
        research_literature_library_native::editable_template::validate_editable_template(value)
            .map_err(|message| Flow::original(crate::ai::error::AiError::InvalidInput(message)))
    }
    type Pause = HbsNeed;
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<crate::ai::error::AiError, HbsNeed>> {
        assert!(!self.panic, "synthetic HBS policy unwind");
        let slot = self.calls;
        self.calls += 1;
        if self.pause_at == Some(slot) {
            return Err(Flow::Need(HbsNeed::Structured {
                slot,
                input: StructuredValidationInputV1 {
                    target: if kind == "portrait" {
                        StructuredTargetV1::Portrait
                    } else {
                        StructuredTargetV1::Overview
                    },
                    candidate_json: value.to_string(),
                    evidence: evidence
                        .items
                        .iter()
                        .map(|e| {
                            research_workbench_native_sdk::candidate_contracts::EvidenceQuote {
                                citation_key: e.citation_key.clone(),
                                quote_exact: e.quote_exact.clone(),
                            }
                        })
                        .collect(),
                },
            }));
        }
        if self.reject_at == Some(slot) {
            return Err(Flow::Fail(Failure {
                error: crate::ai::error::AiError::Validation {
                    path: "$.original".into(),
                    message: "原拒绝".into(),
                },
                cause: FailureCause::NativeRejection(Box::new(origin())),
            }));
        }
        crate::hbs_storage::test_sources::structured(kind, value, evidence).map_err(Flow::original)
    }
}
pub(super) struct Sources {
    pub ready: bool,
    pub requires: usize,
    pub policy: Policy,
    reading: crate::hbs_storage::test_sources::ReadingEvidence,
}
impl Sources {
    pub fn new() -> Self {
        Self {
            ready: true,
            requires: 0,
            policy: Policy {
                calls: 0,
                pause_at: None,
                reject_at: None,
                panic: false,
            },
            reading: crate::hbs_storage::test_sources::ReadingEvidence,
        }
    }
}
impl HbsSources for Sources {
    type Policy = Policy;
    fn policy_only(&mut self) -> Result<&mut Policy, Flow<String, HbsNeed>> {
        Ok(&mut self.policy)
    }
    fn require(
        &mut self,
    ) -> Result<
        crate::ai::structured_resource_validation::ValidationSources<'_, Policy>,
        Flow<String, HbsNeed>,
    > {
        self.requires += 1;
        if !self.ready {
            return Err(Flow::Need(HbsNeed::Sources));
        }
        Ok(
            crate::ai::structured_resource_validation::ValidationSources {
                policy: &mut self.policy,
                reading: &self.reading,
            },
        )
    }
}
pub(super) fn origin() -> crate::ai::structured_resource_validation::DecisionOrigin {
    crate::ai::structured_resource_validation::DecisionOrigin {
        package_key: "a".repeat(64),
        instance_id: "fixture-instance".into(),
        native_request_id: "fixture-decision".into(),
    }
}
pub(super) fn body() -> Value {
    json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1",
        "fields":(["research_question","approach","key_result","conclusion","limitation","open_question"].map(|field|json!({"field":field,"status":"not_found","statements":[]})))})
}
pub(super) fn seeded(db: &mut Connection) -> (String, Value) {
    crate::hbs_storage::test_sources::seed(db, "kernel-paper", body()).unwrap()
}
pub(super) fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}
pub(super) fn request(before: &Value) -> Commit {
    let mut saved = before["saved"].clone();
    saved["fields"][0] = json!({"field":"research_question","status":"filled","statements":[{"id":"s","text":"candidate","epistemicRole":"uncertain","evidenceItemIds":[]}]});
    Commit {
        id: "kernel-request".into(),
        expected_version: before["version"].as_i64().unwrap(),
        saved,
        baseline: before["baseline"].clone(),
    }
}
pub(super) fn checkpoint(snapshot: &Value, content: Value) -> Value {
    json!({"format":1,"kind":"structured","observed":snapshot,"content":content,"ancestor":snapshot["saved"],"reviewAncestor":snapshot["saved"],"baselineAncestor":snapshot["baseline"],"ordinary":[]})
}
pub(super) fn assert_rejected(error: BoundaryError<HbsNeed>, prefix: &str) {
    let BoundaryError::Failure(f) = error else {
        panic!("expected original refusal")
    };
    assert!(f.failure.error.starts_with(prefix), "{}", f.failure.error);
    assert_eq!(f.outcome, HbsOutcome::RolledBack);
    let FailureCause::NativeRejection(actual) = f.failure.cause else {
        panic!("decision origin lost")
    };
    assert_eq!(actual.package_key, origin().package_key);
    assert_eq!(actual.instance_id, origin().instance_id);
    assert_eq!(actual.native_request_id, origin().native_request_id);
}
