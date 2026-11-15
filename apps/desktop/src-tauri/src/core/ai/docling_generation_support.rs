//! Fixed original evidence publication stays in Core; tests reuse the same run writer.
#[path = "docling_generation_support/evidence_store.rs"]
mod evidence_store;
pub(crate) use super::run_state_repository::StageCompletion;
#[cfg(test)]
pub(crate) use evidence_store::persist_docling_evidence;
pub(crate) use evidence_store::{
    DoclingEvidenceIdentity, DoclingEvidenceSource, DoclingStatements, apply_docling_evidence,
};

#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) struct GenerationAccess<'guard, 'owner> {
    db: &'guard mut crate::data_interface::DatabaseGuard<'owner>,
}
#[cfg(all(test, debug_assertions, feature = "development-data"))]
impl<'guard, 'owner> GenerationAccess<'guard, 'owner> {
    pub(crate) fn new(db: &'guard mut crate::data_interface::DatabaseGuard<'owner>) -> Self {
        Self { db }
    }
    pub(crate) fn begin(
        &mut self,
        run: &str,
        kind: &str,
        input: Option<&str>,
    ) -> Result<String, super::error::AiError> {
        super::run_state_repository::begin_stage(self.db, run, kind, input)
    }
    pub(crate) fn complete(
        &self,
        stage: &str,
        completion: StageCompletion<'_>,
    ) -> Result<(), super::error::AiError> {
        super::run_state_repository::complete_stage(self.db, stage, completion)
    }
    pub(crate) fn finish(
        &mut self,
        run: &str,
        status: &str,
        code: Option<&str>,
        message: Option<&str>,
        usage: Option<(i64, i64, i64, i64)>,
    ) -> Result<(), super::error::AiError> {
        super::run_state_repository::finish_run_guarded(self.db, run, status, code, message, usage)
    }
}
