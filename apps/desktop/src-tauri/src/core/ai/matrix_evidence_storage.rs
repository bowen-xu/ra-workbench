//! Original matrix bundle publication on Core's connection and transaction owner.
use super::{
    error::AiError,
    generation_storage_failure::GenerationStorageUnknown,
    structured_resource_validation::Flow,
    task_evidence_support::{
        AiEvidenceBundle, AiEvidencePreview, AnchorEvidence, EvidenceItemInsert, TaskEvidenceFacts,
        TaskEvidenceJournal,
    },
};
use crate::hbs_storage::boundary::{self, BoundaryError};
use rusqlite::Connection;
use std::convert::Infallible;
use uuid::Uuid;

pub(crate) fn persist_matrix_evidence(
    db: &mut Connection,
    run: &str,
    scope: &str,
    sources: &[AnchorEvidence],
    rows: &[AiEvidencePreview],
    explicit_settlement: bool,
) -> Result<AiEvidenceBundle, AiError> {
    let id = Uuid::new_v4().to_string();
    let tx = db.transaction()?;
    let result = (|| {
        let journal = TaskEvidenceJournal::new(&tx);
        journal.append_bundle(&id, run, scope, "matrix-cell-revisions-v1")?;
        for (ordinal, (source, row)) in sources.iter().zip(rows).enumerate() {
            journal.append_item(EvidenceItemInsert {
                bundle: &id,
                citation_key: &row.citation_key,
                ordinal: i64::try_from(ordinal)
                    .map_err(|_| AiError::InvalidInput("综合来源数量超限".into()))?,
                source,
                context: &row.context_text,
                relation: &row.relation_kind,
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
