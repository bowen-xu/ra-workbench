//! Original selected-anchor bundle and item writes in the Core-owned transaction.
use super::{
    error::AiError,
    generation_storage_failure::GenerationStorageUnknown,
    structured_resource_validation::Flow,
    task_evidence_support::{
        AiEvidenceBundle, AnchorEvidence, EvidenceItemInsert, TaskEvidenceFacts,
        TaskEvidenceJournal,
    },
};
use crate::hbs_storage::boundary::{self, BoundaryError};
use rusqlite::Connection;
use std::convert::Infallible;
use uuid::Uuid;
pub(crate) fn persist_selection_evidence(
    db: &mut Connection,
    run: &str,
    scope: &str,
    source: &AnchorEvidence,
    context: &str,
    explicit_settlement: bool,
) -> Result<AiEvidenceBundle, AiError> {
    let id = Uuid::new_v4().to_string();
    let tx = db.transaction()?;
    let result = (|| {
        let journal = TaskEvidenceJournal::new(&tx);
        journal.append_bundle(&id, run, scope, "selection-anchor-v1")?;
        let rows = [
            ("ev-quote", "direct_quote", source.quote_exact.as_str()),
            ("ev-context", "context", context),
        ];
        for (ordinal, (key, relation, context)) in rows.iter().enumerate() {
            journal.append_item(EvidenceItemInsert {
                bundle: &id,
                citation_key: key,
                ordinal: i64::try_from(ordinal)
                    .map_err(|_| AiError::InvalidInput("证据数量超出支持范围".to_owned()))?,
                source,
                context,
                relation,
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
