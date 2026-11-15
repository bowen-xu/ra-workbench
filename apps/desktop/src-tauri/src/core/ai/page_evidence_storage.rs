//! Original page-anchor and evidence writes in the Core caller's transaction.
use super::{
    error::AiError,
    task_evidence_support::{
        AiEvidenceBundle, AnchorEvidence, EvidenceItemInsert, TaskEvidenceFacts,
        TaskEvidenceJournal,
    },
};
use super::{
    generation_storage_failure::GenerationStorageUnknown, structured_resource_validation::Flow,
};
use crate::hbs_storage::boundary::{self, BoundaryError};
use research_workbench_native_sdk::generation::evidence::{
    PAGE_EVIDENCE_CONFIG_VERSION, PreparedPageEvidence,
};
use rusqlite::{Connection, Transaction, params};
use std::convert::Infallible;
use uuid::Uuid;

pub(crate) fn persist_page_evidence(
    db: &mut Connection,
    run: &str,
    scope: &str,
    rows: &[PreparedPageEvidence],
    explicit_settlement: bool,
) -> Result<AiEvidenceBundle, AiError> {
    let id = Uuid::new_v4().to_string();
    let tx = db.transaction()?;
    let result = (|| {
        let writer = PageEvidenceWrite { tx: &tx };
        writer
            .journal()
            .append_bundle(&id, run, scope, PAGE_EVIDENCE_CONFIG_VERSION)?;
        for (ordinal, prepared) in rows.iter().enumerate() {
            let source = &prepared.source;
            writer.anchor(source)?;
            let row = AnchorEvidence {
                anchor_id: source.anchor_id.clone(),
                paper_id: source.paper_id.clone(),
                paper_title: source.paper_title.clone(),
                attachment_id: source.attachment_id.clone(),
                content_sha256: source.content_sha256.clone(),
                page_index: source.page_index,
                page_label: Some((source.page_index + 1).to_string()),
                quote_exact: source.quote_exact.clone(),
                quote_prefix: source.quote_prefix.clone(),
                quote_suffix: source.quote_suffix.clone(),
            };
            writer.journal().append_item(EvidenceItemInsert {
                bundle: &id,
                citation_key: &prepared.citation_key,
                ordinal: i64::try_from(ordinal)
                    .map_err(|_| AiError::InvalidInput("概要证据数量超出支持范围".to_owned()))?,
                source: &row,
                context: &prepared.context,
                relation: "direct_quote",
            })?;
        }
        Ok(())
    })()
    .map_err(Flow::<AiError, Infallible>::original);
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        explicit_settlement,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    match boundary::confirm_with(db, settled, explicit_settlement, |_| {
        AiError::TemporaryUnavailable
    }) {
        Ok(()) => (),
        Err(BoundaryError::Failure(failure)) if explicit_settlement && failure.needs_recovery() => {
            return Err(AiError::GenerationStorageUnknown(Box::new(
                GenerationStorageUnknown::from_failure(run, "evidence_bundle", Some(id), failure),
            )));
        }
        Err(error) => return Err(error.into_static()),
    }
    TaskEvidenceFacts::new(db).load_bundle(&id)
}

struct PageEvidenceWrite<'tx, 'db> {
    tx: &'tx Transaction<'db>,
}
impl PageEvidenceWrite<'_, '_> {
    fn journal(&self) -> TaskEvidenceJournal<'_, '_> {
        TaskEvidenceJournal::new(self.tx)
    }
    fn anchor(
        &self,
        source: &research_workbench_native_sdk::generation::evidence::OverviewPageEvidence,
    ) -> Result<(), AiError> {
        self.tx.execute(
            "insert into source_anchors
         (id, item_id, attachment_id, content_sha256, selector_version, page_index,
          page_label, rects_json, quote_exact, quote_prefix, quote_suffix, created_at)
         values (?1, ?2, ?3, ?4, 1, ?5, ?6, '[]', ?7, ?8, ?9,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(id) do nothing",
            params![
                source.anchor_id,
                source.paper_id,
                source.attachment_id,
                source.content_sha256,
                source.page_index,
                (source.page_index + 1).to_string(),
                source.quote_exact,
                source.quote_prefix,
                source.quote_suffix
            ],
        )?;
        Ok(())
    }
}
