//! Explicit accepted/rejected policy responses for Core transaction tests.
use crate::ai::{
    candidate_contracts::AiEvidenceBundle,
    error::AiError,
    paper_reading_contracts::SnapshotEvidenceRow,
    structured_resource_validation::{
        ExplicitReadingEvidence, Flow, ReadingValidationEvidence, SourceReadFailure,
        StructuredPolicy, ValidationSources,
    },
};
use crate::hbs_storage::boundary::{BoundaryError, HbsSources};
use research_workbench_native_sdk::review_selection::UnreviewedSelectionInputV1;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::convert::Infallible;

#[derive(Default)]
pub(crate) struct Policy {
    rejected_value: Option<(Value, String, String)>,
}
impl StructuredPolicy for Policy {
    type Pause = Infallible;
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<AiError, Infallible>> {
        Ok(value)
    }
    fn validate(
        &mut self,
        _: &str,
        value: Value,
        _: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, Infallible>> {
        if let Some((rejected, path, message)) = &self.rejected_value {
            if let Some(kind) = rejected.get("kind") {
                assert_eq!(&value["kind"], kind);
            }
            return Err(Flow::original(AiError::Validation {
                path: path.clone(),
                message: message.clone(),
            }));
        }
        Ok(accepted_response(value))
    }
}
pub(crate) fn accepted_response(value: Value) -> Value {
    let payload: research_workbench_native_sdk::candidate_contracts::CandidatePayload =
        serde_json::from_value(value).expect("typed accepted structured fixture response");
    serde_json::to_value(payload).expect("serialize public structured response")
}
impl crate::structured_resources::ReviewPolicy for Policy {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, Infallible> {
        Ok(unreviewed_selection(&input))
    }
}
pub(crate) fn unreviewed_selection(input: &UnreviewedSelectionInputV1) -> Vec<String> {
    let cases: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/review-selection-v1/cases.json"
    )))
    .unwrap();
    let expected = serde_json::json!({"pending":input.pending_field_ids,"protected":input.protected_field_ids,"reviewed":input.reviewed_field_ids});
    let observed = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["input"] == expected)
        .unwrap_or_else(|| panic!("missing authored selection response: {expected}"));
    serde_json::from_value(observed["selected"].clone()).unwrap()
}

pub(crate) struct ReadingEvidence;
impl ExplicitReadingEvidence for ReadingEvidence {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        db.query_row(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/native-resources-v1/literature/reading-evidence-snapshot-hash.v1.sql")),
            [snapshot, paper], |row| row.get(0)).optional().map_err(|_| SourceReadFailure::Static)
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        let mut query = db.prepare(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/native-resources-v1/literature/reading-evidence-snapshot-rows.v1.sql")))
            .map_err(|_| SourceReadFailure::Static)?;
        query
            .query_map([snapshot], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|_| SourceReadFailure::Static)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SourceReadFailure::Static)
    }
}
impl ReadingValidationEvidence for ReadingEvidence {
    fn validation_latest_snapshot(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        db.query_row(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/native-resources-v1/literature/reading-evidence-validation-latest.v1.sql")),
            [paper], |row| row.get(0)).optional().map_err(|_| SourceReadFailure::Static)
    }
}

pub(crate) struct Sources {
    policy: Policy,
    reading: ReadingEvidence,
}
impl Sources {
    pub(crate) fn rejecting(value: Value) -> Self {
        Self::rejecting_at(value, "$", "synthetic structured domain rejection")
    }
    pub(crate) fn rejecting_at(value: Value, path: &str, message: &str) -> Self {
        Self {
            policy: Policy {
                rejected_value: Some((value, path.into(), message.into())),
            },
            reading: ReadingEvidence,
        }
    }
    pub(crate) fn new() -> Self {
        Self {
            policy: Policy::default(),
            reading: ReadingEvidence,
        }
    }
}
impl HbsSources for Sources {
    type Policy = Policy;
    fn policy_only(&mut self) -> Result<&mut Policy, Flow<String, Infallible>> {
        Ok(&mut self.policy)
    }
    fn require(&mut self) -> Result<ValidationSources<'_, Policy>, Flow<String, Infallible>> {
        Ok(ValidationSources {
            policy: &mut self.policy,
            reading: &self.reading,
        })
    }
}

pub(crate) fn seed(
    db: &mut Connection,
    paper: &str,
    value: Value,
) -> Result<(String, Value), String> {
    crate::structured_resources::hbs::seed_with(&mut Sources::new(), db, paper, value)
        .map_err(Flow::into_static)
}
pub(crate) fn commit(
    db: &mut Connection,
    kind: &str,
    id: &str,
    request: super::Commit,
) -> Result<Value, String> {
    super::commit_with(&mut Sources::new(), db, kind, id, &request)
        .map_err(BoundaryError::into_static)
}
pub(crate) fn preservation_read(db: &Connection, kind: &str, id: &str) -> Result<Value, String> {
    super::preservation::read_with(&mut Sources::new(), db, kind, id)
        .map_err(BoundaryError::into_static)
}
pub(crate) fn preservation_save(
    db: &mut Connection,
    kind: &str,
    id: &str,
    value: Value,
) -> Result<Value, String> {
    super::preservation::save_with(&mut Sources::new(), db, kind, id, value)
        .map_err(BoundaryError::into_static)
}
pub(crate) fn write(
    db: &mut Connection,
    input: crate::structured_resources::WriteDraft,
) -> Result<crate::structured_resources::WriteResult, crate::structured_resources::Error> {
    crate::structured_resources::write_with(&mut Sources::new(), db, input)
        .map_err(BoundaryError::into_static)
}
pub(crate) fn review(
    db: &mut Connection,
    input: crate::structured_resources::ReviewDraft,
) -> Result<crate::structured_resources::Resource, crate::structured_resources::Error> {
    // Core's ordinary manual route includes explicit field verification.
    crate::structured_resources::review_with(
        &mut Sources::new(),
        db,
        input,
        crate::structured_resources::ReviewRoute::Static,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn adopt(
    db: &mut Connection,
    input: crate::structured_resources::AdoptionDraft,
) -> Result<crate::structured_resources::Resource, crate::structured_resources::Error> {
    crate::structured_resources::adopt_template_with(&mut Sources::new(), db, input)
        .map_err(BoundaryError::into_static)
}

pub(crate) fn restore(
    db: &mut Connection,
    input: crate::structured_resources::HistoryDraft,
) -> Result<crate::structured_resources::Resource, crate::structured_resources::Error> {
    crate::structured_resources::restore_history_with(
        &mut Sources::new(),
        db,
        input,
        crate::structured_resources::ReviewRoute::Static,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn patch(
    db: &mut Connection,
    input: crate::structured_resources::PatchDraft,
) -> Result<crate::structured_resources::WriteResult, crate::structured_resources::Error> {
    crate::structured_resources::patch_with(&mut Sources::new(), db, input)
        .map_err(BoundaryError::into_static)
}
