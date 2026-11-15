//! Fixed shared candidate/resource and historical-run facts for template policy.
use rusqlite::{Connection, OptionalExtension, params};

use super::{candidate_contracts::PaperTemplateRef, error::AiError, model::AiCandidate};
use research_workbench_native_sdk::analysis_workflow::template::{
    TemplateEvidenceV1, TemplateInputV1, TemplateRevisionV1,
};

pub(crate) struct TemplateFacts<'db> {
    db: &'db Connection,
}
impl<'db> TemplateFacts<'db> {
    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }

    pub(crate) fn managed_resources(&self) -> Result<bool, AiError> {
        self.db.query_row("select exists(select 1 from sqlite_master where type='table' and name='structured_resources')",[],|row|row.get(0)).map_err(Into::into)
    }

    pub(crate) fn latest_portrait(
        &self,
        paper_id: &str,
        kind: &str,
    ) -> Result<Option<(String, i64)>, AiError> {
        let confirmed: Option<(String, i64)> = self.db.query_row(
            "select d.candidate_id, d.candidate_revision from ai_review_decisions d
         join ai_candidates c on c.id = d.candidate_id
         join ai_evidence_bundles b on b.id = c.evidence_bundle_id
         where c.candidate_kind = ?2 and d.action in ('accept_unchanged', 'accept_after_edit')
           and c.validation_status = 'valid' and c.review_state != 'rejected'
           and exists(select 1 from ai_evidence_items e where e.bundle_id = b.id and e.paper_id = ?1)
           and not exists(select 1 from ai_evidence_items e where e.bundle_id = b.id and e.paper_id != ?1)
         order by d.created_at desc, d.id desc limit 1", params![paper_id,kind], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        match confirmed {
            Some(value) => Ok(Some(value)),
            None => self.db.query_row(
                "select c.id, c.current_revision from ai_candidates c
             join ai_evidence_bundles b on b.id = c.evidence_bundle_id
             where c.candidate_kind = 'paper_portrait_v1' and c.validation_status = 'valid'
               and c.review_state in ('unreviewed', 'edited')
               and exists(select 1 from ai_evidence_items e where e.bundle_id = b.id and e.paper_id = ?1)
               and not exists(select 1 from ai_evidence_items e where e.bundle_id = b.id and e.paper_id != ?1)
             order by c.created_at desc, c.id desc limit 1", [paper_id], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(Into::into),
        }
    }

    pub(crate) fn candidate(&self, id: &str) -> Result<AiCandidate, AiError> {
        super::candidate_repository::get_candidate(self.db, id)
    }

    pub(crate) fn legacy_content(
        &self,
        paper_id: &str,
        reference: &PaperTemplateRef,
    ) -> Result<TemplateInputV1, AiError> {
        let candidate = self.candidate(reference.id())?;
        Ok(TemplateInputV1::LegacyContent {
            paper_id: paper_id.into(),
            reference: reference.clone(),
            current_revision: candidate.current_revision,
            evidence: candidate
                .evidence_bundle
                .items
                .into_iter()
                .map(|item| TemplateEvidenceV1 {
                    paper_id: item.paper_id,
                    current_anchor_status: item.current_anchor_status,
                })
                .collect(),
            revisions: candidate
                .revisions
                .into_iter()
                .map(|revision| TemplateRevisionV1 {
                    revision: revision.revision,
                    payload: revision.payload,
                })
                .collect(),
        })
    }

    pub(crate) fn reading_template(
        &self,
        paper: &str,
    ) -> Result<(i64, Option<(String, i64)>), AiError> {
        let reading =
            crate::structured_resources::find_by_paper(self.db, paper, "structuredReading")
                .map_err(|_| AiError::InvalidInput("结构化阅读资源不可用".into()))?;
        Ok((
            reading.revision,
            reading
                .template_ref
                .map(|reference| (reference.resource_id, reference.revision)),
        ))
    }
    pub(crate) fn resource_revision(
        &self,
        reference: &PaperTemplateRef,
    ) -> Result<String, AiError> {
        self.db.query_row("select v.content_json from structured_resource_revisions v join structured_resources r on r.id=v.resource_id where r.kind='readingTemplate' and v.resource_id=?1 and v.revision=?2",params![reference.id(),reference.revision()],|row|row.get(0)).map_err(Into::into)
    }
}
