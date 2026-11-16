//! Test adapters for the same generic Core boundary and canonical Native rules.
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
use research_workbench_native_sdk::{
    candidate_contracts::EvidenceQuote, review_selection::UnreviewedSelectionInputV1,
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::convert::Infallible;

pub(crate) struct Policy;
impl StructuredPolicy for Policy {
    type Pause = Infallible;
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<AiError, Infallible>> {
        research_literature_library_native::editable_template::validate_editable_template(value)
            .map_err(|message| Flow::original(AiError::InvalidInput(message)))
    }
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, Infallible>> {
        structured(kind, value, evidence).map_err(Flow::original)
    }
}
impl crate::structured_resources::ReviewPolicy for Policy {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, Infallible> {
        Ok(
            research_literature_library_native::review_selection::unreviewed_fields(
                &input.pending_field_ids,
                &input.protected_field_ids,
                &input.reviewed_field_ids,
            ),
        )
    }
}
pub(crate) fn structured(
    kind: &str,
    value: Value,
    evidence: &AiEvidenceBundle,
) -> Result<Value, AiError> {
    let quotes: Vec<_> = evidence
        .items
        .iter()
        .map(|item| EvidenceQuote {
            citation_key: item.citation_key.clone(),
            quote_exact: item.quote_exact.clone(),
        })
        .collect();
    research_literature_library_native::structured_validation::structured(kind, value, &quotes)
        .map_err(AiError::from)
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
            "/../../../tests/fixtures/context-resources-v1/literature_library/reading-evidence-snapshot-hash.v1.sql")),
            [snapshot, paper], |row| row.get(0)).optional().map_err(|_| SourceReadFailure::Static)
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        let mut query = db.prepare(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/context-resources-v1/literature_library/reading-evidence-snapshot-rows.v1.sql")))
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
            "/../../../tests/fixtures/context-resources-v1/literature_library/reading-evidence-validation-latest.v1.sql")),
            [paper], |row| row.get(0)).optional().map_err(|_| SourceReadFailure::Static)
    }
}

pub(crate) struct Sources {
    policy: Policy,
    reading: ReadingEvidence,
}
impl Sources {
    pub(crate) fn new() -> Self {
        Self {
            policy: Policy,
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
