//! The original editable-resource checks with narrow, synchronous source seams.
use super::paper_reading_source::PaperReadingSource;
use super::{
    error::AiError,
    model::{AiEvidenceBundle, AiEvidenceItem},
};
use rusqlite::Connection;
use serde_json::Value;
use std::convert::Infallible;

#[path = "structured_resource_validation/explicit.rs"]
mod explicit;
#[path = "structured_resource_validation/flow.rs"]
mod flow;
pub(crate) use explicit::{
    ExplicitEvidenceError, ExplicitReadingEvidence, SourceReadFailure, explicit_evidence,
};
pub(crate) use flow::{DecisionOrigin, Failure, FailureCause, Flow};

pub(crate) trait StructuredPolicy {
    type Pause;
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<AiError, Self::Pause>>;
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, Self::Pause>>;
}
pub(crate) trait ReadingValidationEvidence: ExplicitReadingEvidence {
    fn validation_latest_snapshot(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure>;
}
pub(crate) struct ValidationSources<'a, P: StructuredPolicy> {
    pub(crate) policy: &'a mut P,
    pub(crate) reading: &'a dyn ReadingValidationEvidence,
}
impl StructuredPolicy for crate::ai::generation_policy::GenerationPolicySource {
    type Pause = Infallible;
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<AiError, Infallible>> {
        (self.editable_template)(value)
            .map_err(|message| Flow::original(AiError::InvalidInput(message)))
    }
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, Self::Pause>> {
        (self.structured)(kind, value, evidence).map_err(Flow::original)
    }
}
impl ReadingValidationEvidence for PaperReadingSource {
    fn validation_latest_snapshot(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        (self.validation_latest_snapshot)(db, paper).map_err(|_| SourceReadFailure::Static)
    }
}

pub(crate) fn validate_with<P: StructuredPolicy>(
    sources: &mut ValidationSources<'_, P>,
    db: &Connection,
    paper: &str,
    kind: &str,
    value: Value,
    source: Option<&str>,
) -> Result<Value, Flow<AiError, P::Pause>> {
    let bundle = evidence_with(sources.reading, db, paper, source)
        .map_err(|failure| Flow::Fail(failure.map_error(AiError::InvalidInput)))?;
    sources.policy.validate(kind, value, &bundle)
}
pub(crate) fn validate_many_with<P: StructuredPolicy>(
    sources: &mut ValidationSources<'_, P>,
    db: &Connection,
    paper: &str,
    kind: &str,
    value: Value,
    source_ids: &[String],
) -> Result<Value, Flow<AiError, P::Pause>> {
    if source_ids.is_empty() {
        return validate_with(sources, db, paper, kind, value, None);
    }
    let mut bundle = evidence_with(sources.reading, db, paper, Some(&source_ids[0]))
        .map_err(|failure| Flow::Fail(failure.map_error(AiError::InvalidInput)))?;
    for source in &source_ids[1..] {
        bundle.items.extend(
            evidence_with(sources.reading, db, paper, Some(source))
                .map_err(|failure| Flow::Fail(failure.map_error(AiError::InvalidInput)))?
                .items,
        );
    }
    sources.policy.validate(kind, value, &bundle)
}
pub(crate) fn evidence_with(
    reading: &dyn ReadingValidationEvidence,
    db: &Connection,
    paper: &str,
    source: Option<&str>,
) -> Result<AiEvidenceBundle, Failure<String>> {
    if let Some(source) = source {
        return explicit::explicit_evidence_for(reading, db, paper, source)
            .map_err(ExplicitEvidenceError::into_failure);
    }
    let snapshot = reading
        .validation_latest_snapshot(db, paper)
        .map_err(|cause| ExplicitEvidenceError::read("无法读取文献来源", cause).into_failure())?;
    let mut items = Vec::new();
    if let Some(snapshot) = &snapshot {
        let rows = reading
            .snapshot_evidence_rows(db, snapshot)
            .map_err(|cause| {
                ExplicitEvidenceError::read("无法读取冻结原文", cause).into_failure()
            })?;
        for (id, attachment, sha, page, text, current_sha) in rows {
            let status = anchor_status(&current_sha, &sha);
            let mut source = item(paper, id, attachment, sha, page, text);
            source.current_anchor_status = status.into();
            items.push(source);
        }
    }
    // Existing manual edits retain their immutable history; explicit sources never widen.
    if source.is_none() {
        let mut stmt=db.prepare("select citation_key,attachment_id,content_sha256,page_index,quote_exact from ai_evidence_items where paper_id=?1 order by created_at,id").map_err(|error|Failure::sql("无法读取历史原文".into(),"source-read",&error))?;
        let rows = stmt
            .query_map([paper], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| Failure::sql("无法读取历史原文".into(), "source-read", &error))?;
        for row in rows {
            let (id, attachment, sha, page, text) = row
                .map_err(|error| Failure::sql("无法读取历史原文".into(), "source-read", &error))?;
            items.push(item(paper, id, attachment, sha, page, text));
        }
    }
    Ok(AiEvidenceBundle {
        id: snapshot.unwrap_or_default(),
        run_id: String::new(),
        scope_hash: String::new(),
        retrieval_config_version: "structured-resource-v1".into(),
        validation_status: "valid".into(),
        created_at: String::new(),
        items,
    })
}
fn anchor_status(current: &str, frozen: &str) -> &'static str {
    if current.is_empty() {
        "missing"
    } else if current != frozen {
        "stale"
    } else {
        "page_level"
    }
}
#[cfg(test)]
#[test]
fn frozen_anchor_status_distinguishes_missing_replaced_and_unchanged_content() {
    assert_eq!(anchor_status("", "old"), "missing");
    assert_eq!(anchor_status("new", "old"), "stale");
    assert_eq!(anchor_status("old", "old"), "page_level");
}
fn item(
    paper: &str,
    id: String,
    attachment: String,
    sha: String,
    page: i64,
    text: String,
) -> AiEvidenceItem {
    AiEvidenceItem {
        id: id.clone(),
        citation_key: id,
        ordinal: 0,
        anchor_id: String::new(),
        paper_id: paper.into(),
        paper_title: String::new(),
        attachment_id: attachment,
        content_sha256: sha,
        page_index: page,
        page_label: None,
        quote_exact: text,
        quote_prefix: String::new(),
        quote_suffix: String::new(),
        context_text: String::new(),
        relation_kind: "context".into(),
        validation_status: "valid".into(),
        current_anchor_status: "page_level".into(),
        created_at: String::new(),
    }
}
