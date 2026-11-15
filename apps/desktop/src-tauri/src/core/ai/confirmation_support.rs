//! Closed shared-candidate facts and append-only confirmation journal. Borrowed
//! handles expose no connection, transaction owner or domain execution policy.
pub(crate) use super::model::{
    AiCandidate, AiOverviewConfirmationBatch, AiOverviewConfirmationResult,
    AiOverviewConfirmationUndoDraft, AiOverviewFieldConfirmation, AiOverviewFieldConfirmationDraft,
    AiPaperReviewConfirmationBatch, AiPaperReviewConfirmationDraft,
    AiPaperReviewConfirmationResult, AiPaperReviewConfirmationUndoDraft,
    AiPaperReviewMetadataConfirmation, AiPaperReviewOverviewTargetDraft,
};
use super::{candidate_repository, error::AiError, overview_confirmation_repository as ledger};
pub(crate) use ledger::{BatchInsert, FieldInsert};
use rusqlite::{Connection, Transaction};

pub(crate) struct ConfirmationFacts<'db> {
    db: &'db Connection,
}
impl<'db> ConfirmationFacts<'db> {
    /// Internal native repository composition on its existing guard/snapshot.
    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }
    pub(crate) fn candidate(&self, id: &str) -> Result<AiCandidate, AiError> {
        candidate_repository::get_candidate(self.db, id)
    }
    pub(crate) fn require_completed_turn(&self, id: &str) -> Result<(), AiError> {
        candidate_repository::ensure_shared_turn_completed_for_candidate(self.db, id)
    }
    pub(crate) fn stored_review_state(&self, id: &str) -> Result<String, AiError> {
        ledger::stored_review_state(self.db, id)
    }
    pub(crate) fn batch_for_key(&self, key: &str) -> Result<Option<String>, AiError> {
        ledger::batch_for_key(self.db, key)
    }
    pub(crate) fn batch(&self, id: &str) -> Result<AiOverviewConfirmationBatch, AiError> {
        ledger::load_confirmation_batch(self.db, id)
    }
    pub(crate) fn fields(&self, id: &str) -> Result<Vec<AiOverviewFieldConfirmation>, AiError> {
        ledger::list_overview_field_confirmations(self.db, id)
    }
    pub(crate) fn reset_key_exists(&self, key: &str) -> Result<bool, AiError> {
        ledger::reset_key_exists(self.db, key)
    }
    pub(crate) fn undo_for_key(&self, key: &str) -> Result<Option<String>, AiError> {
        ledger::undo_for_key(self.db, key)
    }
    pub(crate) fn reset_for_key(&self, key: &str) -> Result<Option<String>, AiError> {
        ledger::reset_for_key(self.db, key)
    }
    pub(crate) fn undo_key_exists(&self, key: &str) -> Result<bool, AiError> {
        ledger::undo_key_exists(self.db, key)
    }
    pub(crate) fn batch_is_active(&self, id: &str) -> Result<bool, AiError> {
        ledger::is_batch_active(self.db, id)
    }
    pub(crate) fn previous_snapshot(&self, id: &str) -> Result<Option<String>, AiError> {
        ledger::previous_snapshot(self.db, id)
    }
    pub(crate) fn previous_state(&self, id: &str) -> Result<String, AiError> {
        ledger::previous_state(self.db, id)
    }
    pub(crate) fn distinct_active_fields(&self, id: &str) -> Result<i64, AiError> {
        ledger::active_count(self.db, id, ledger::ActiveCount::DistinctFields)
    }
    pub(crate) fn active_confirmations(&self, id: &str) -> Result<i64, AiError> {
        ledger::active_count(self.db, id, ledger::ActiveCount::Confirmations)
    }
    pub(crate) fn has_human_edit(&self, id: &str) -> Result<bool, AiError> {
        ledger::has_human(self.db, id)
    }
}

pub(crate) struct ConfirmationJournal<'tx, 'db> {
    transaction: &'tx Transaction<'db>,
}
impl<'tx, 'db> ConfirmationJournal<'tx, 'db> {
    /// Borrow the caller's exact transaction; never commit or acquire a guard.
    pub(crate) fn new(transaction: &'tx Transaction<'db>) -> Self {
        Self { transaction }
    }
    pub(crate) fn facts(&self) -> ConfirmationFacts<'_> {
        ConfirmationFacts::new(self.transaction)
    }
    pub(crate) fn append_batch(&self, row: BatchInsert<'_>) -> Result<(), AiError> {
        ledger::append_batch(self.transaction, row)
    }
    pub(crate) fn append_field(&self, row: FieldInsert<'_>) -> Result<(), AiError> {
        ledger::append_field(self.transaction, row)
    }
    pub(crate) fn append_undo(&self, id: &str, batch: &str, key: &str) -> Result<(), AiError> {
        ledger::append_undo(self.transaction, id, batch, key)
    }
    pub(crate) fn append_reset(&self, id: &str, field: &str, key: &str) -> Result<(), AiError> {
        ledger::append_reset(self.transaction, id, field, key)
    }
    pub(crate) fn set_review_state(&self, candidate: &str, state: &str) -> Result<(), AiError> {
        ledger::set_review_state(self.transaction, candidate, state)
    }
}
