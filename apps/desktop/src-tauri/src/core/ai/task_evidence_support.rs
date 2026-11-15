//! Fixed run/index facts and shared evidence publication on the caller's guard.
//! Selection, excerpting, source-anchor policy and budgets belong to the domain.
pub(crate) use super::matrix_evidence_storage::persist_matrix_evidence;
pub(crate) use super::model::{
    AiEvidenceBundle, AiEvidencePreview, AiMatrixSynthesisContext, AiMatrixSynthesisContextDraft,
    AiScopeItem,
};
pub(crate) use super::page_evidence_storage::persist_page_evidence;
pub(crate) use super::selection_evidence_storage::persist_selection_evidence;
use super::{error::AiError, evidence_repository, matrix_synthesis_context_repository};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) enum TaskEvidenceKind {
    Selection,
    Pages,
    Matrix,
}
pub(crate) enum TaskEvidencePreview<'a> {
    Selection(
        #[expect(
            dead_code,
            reason = "The existing evidence-preview port retains its borrowed source identities."
        )]
        &'a str,
    ),
    Pages(
        #[expect(
            dead_code,
            reason = "The existing evidence-preview port retains its borrowed source identities."
        )]
        &'a str,
    ),
    Matrix(
        #[expect(
            dead_code,
            reason = "The existing evidence-preview port retains its borrowed source identities."
        )]
        &'a [String],
    ),
}
pub(crate) use research_workbench_native_sdk::generation::evidence::AnchorEvidence;
pub(crate) use research_workbench_native_sdk::generation::evidence::AttachmentEvidence;
pub(crate) struct TaskEvidenceFacts<'db> {
    db: &'db Connection,
}
impl<'db> TaskEvidenceFacts<'db> {
    pub(crate) fn anchor(&self, id: &str) -> Result<Option<AnchorEvidence>, AiError> {
        Ok(self
            .db
            .query_row(
                "select s.id, s.item_id, b.title, s.attachment_id, s.content_sha256,
                    s.page_index, s.page_label, s.quote_exact, s.quote_prefix, s.quote_suffix
             from source_anchors s
             join attachments a on a.id = s.attachment_id and a.content_sha256 = s.content_sha256
             join bibliographic_items b on b.id = s.item_id
             where s.id = ?1",
                [id],
                |row| {
                    Ok(AnchorEvidence {
                        anchor_id: row.get(0)?,
                        paper_id: row.get(1)?,
                        paper_title: row.get(2)?,
                        attachment_id: row.get(3)?,
                        content_sha256: row.get(4)?,
                        page_index: row.get(5)?,
                        page_label: row.get(6)?,
                        quote_exact: row.get(7)?,
                        quote_prefix: row.get(8)?,
                        quote_suffix: row.get(9)?,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn attachment(&self, id: &str) -> Result<Option<AttachmentEvidence>, AiError> {
        Ok(self
            .db
            .query_row(
                "select a.item_id, b.title, a.content_sha256, coalesce(j.state, '')
             from attachments a join bibliographic_items b on b.id = a.item_id
             left join full_text_index_jobs j on j.attachment_id = a.id where a.id = ?1",
                [id],
                |row| {
                    Ok(AttachmentEvidence {
                        paper_id: row.get(0)?,
                        paper_title: row.get(1)?,
                        content_sha256: row.get(2)?,
                        index_state: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }
    pub(crate) fn existing_bundle(&self, run: &str) -> Result<Option<String>, AiError> {
        Ok(self
            .db
            .query_row(
                "select id from ai_evidence_bundles where run_id = ?1",
                [run],
                |row| row.get(0),
            )
            .optional()?)
    }
    pub(crate) fn load_bundle(&self, id: &str) -> Result<AiEvidenceBundle, AiError> {
        evidence_repository::load_bundle(self.db, id)
    }
    pub(crate) fn selection_scope(&self, run: &str) -> Result<(String, String), AiError> {
        self.db
            .query_row(
                "select r.scope_hash, s.object_id
             from ai_runs r
             join ai_scope_items s on s.run_id = r.id
             where r.id = ?1 and r.task_kind in (
                 'selection_explanation', 'reference_extraction'
             )
               and s.item_kind = 'anchor' and s.ordinal = 0",
                [run],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| AiError::InvalidInput("AI 锚点任务缺少原文来源".to_owned()))
    }
    pub(crate) fn pages_scope(
        &self,
        run: &str,
    ) -> Result<(String, Vec<(String, String)>), AiError> {
        let rows=self.db.prepare(
            "select r.scope_hash, s.object_id, s.content_sha256
         from ai_runs r join ai_scope_items s on s.run_id = r.id
         where r.id = ?1 and r.task_kind in (
             'paper_overview', 'paper_concept_map', 'paper_suggestions', 'paper_profile_classification'
         ) and s.item_kind = 'attachment'
           and (r.task_kind = 'paper_profile_classification' or s.ordinal = 0)
         order by s.ordinal"
        )?.query_map([run],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        let scope = rows
            .first()
            .map(|row| row.0.clone())
            .ok_or_else(|| AiError::InvalidInput("论文阅读运行缺少 PDF 附件范围".to_owned()))?;
        Ok((
            scope,
            rows.into_iter().map(|(_, id, sha)| (id, sha)).collect(),
        ))
    }
    pub(crate) fn page_texts(&self, attachment: &str) -> Result<Vec<(i64, String)>, AiError> {
        Ok(self
            .db
            .prepare(
                "select page_index, page_text from attachment_page_texts
         where attachment_id = ?1 and length(trim(page_text)) > 0 order by page_index",
            )?
            .query_map([attachment], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }
    pub(crate) fn matrix_scope_hash(&self, run: &str) -> Result<String, AiError> {
        self.db
            .query_row(
                "select scope_hash from ai_runs
             where id = ?1 and task_kind = 'matrix_synthesis'",
                [run],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| AiError::InvalidInput("多文献综合运行不存在".to_owned()))
    }
    pub(crate) fn matrix_context(
        &self,
        run: &str,
    ) -> Result<Option<AiMatrixSynthesisContext>, AiError> {
        matrix_synthesis_context_repository::load_matrix_synthesis_context(self.db, run)
    }
    pub(crate) fn frozen_anchor_hash(
        &self,
        run: &str,
        anchor: &str,
    ) -> Result<Option<String>, AiError> {
        Ok(self
            .db
            .query_row(
                "select content_sha256 from ai_scope_items
                 where run_id = ?1 and item_kind = 'anchor' and object_id = ?2",
                params![run, anchor],
                |row| row.get(0),
            )
            .optional()?)
    }
}

pub(crate) struct EvidenceItemInsert<'a> {
    pub bundle: &'a str,
    pub citation_key: &'a str,
    pub ordinal: i64,
    pub source: &'a AnchorEvidence,
    pub context: &'a str,
    pub relation: &'a str,
}
pub(crate) struct TaskEvidenceJournal<'tx, 'db> {
    tx: &'tx Transaction<'db>,
}
impl<'tx, 'db> TaskEvidenceJournal<'tx, 'db> {
    pub(crate) fn new(tx: &'tx Transaction<'db>) -> Self {
        Self { tx }
    }
    pub(crate) fn append_bundle(
        &self,
        id: &str,
        run: &str,
        scope: &str,
        config: &str,
    ) -> Result<(), AiError> {
        self.tx.execute(
            "insert into ai_evidence_bundles
         (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
         values (?1, ?2, ?3, ?4, 'valid', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![id, run, scope, config],
        )?;
        Ok(())
    }
    pub(crate) fn append_item(&self, row: EvidenceItemInsert<'_>) -> Result<(), AiError> {
        let source = row.source;
        self.tx.execute(
            "insert into ai_evidence_items
             (id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title,
              attachment_id, content_sha256, page_index, page_label, quote_exact,
              quote_prefix, quote_suffix, context_text, relation_kind, validation_status,
              created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, 'valid', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                Uuid::new_v4().to_string(),
                row.bundle,
                row.citation_key,
                row.ordinal,
                source.anchor_id,
                source.paper_id,
                source.paper_title,
                source.attachment_id,
                source.content_sha256,
                source.page_index,
                source.page_label,
                source.quote_exact,
                source.quote_prefix,
                source.quote_suffix,
                row.context,
                row.relation
            ],
        )?;
        Ok(())
    }
}
