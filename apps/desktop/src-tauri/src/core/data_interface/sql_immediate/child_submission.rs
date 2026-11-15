//! Read a scoped child submission snapshot under the data owner.
use super::{Database, ReadFence, SqlImmediateFailure, diagnostic, receipts};
use rusqlite::params;
use uuid::Uuid;

pub(crate) struct ChildSubmissionInspection {
    pub epoch: Uuid,
    pub record: Option<(Uuid, Uuid)>,
}

impl Database {
    /// A retained row is a snapshot, not proof that the child ran or may resume.
    pub(crate) fn inspect_child_submission(
        &self,
        parent: Uuid,
        child: Uuid,
        request: Uuid,
        binding: &research_agent_core::conversation::ConversationBinding,
    ) -> Result<ChildSubmissionInspection, SqlImmediateFailure> {
        if parent.is_nil() || child.is_nil() || request.is_nil() {
            return Err(SqlImmediateFailure::Domain(
                research_agent_core::conversation::CatalogError::InvalidInput,
            ));
        }
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| super::unmanaged(request))?;
        let inspection = owner
            .inspection_guard()
            .map_err(|error| super::admission_error(request, error, None))?;
        let snapshot = inspection.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                request,
                fence,
                "child-submission-status",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() || snapshot.blocked || snapshot.closed {
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                request,
                fence,
                "child-submission-status",
                "submission_owner_unconfirmed",
            ))));
        }
        let fence = receipts::status_fence(&db, request, fence, snapshot.accepted_position)?;
        if snapshot.accepted_position != fence.position {
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                request,
                fence,
                "child-submission-status",
                "submission_pending_history_unconfirmed",
            ))));
        }
        use research_agent_core::conversation::BindingKind;
        let kind = match binding.kind {
            BindingKind::Manuscript => "manuscript",
            BindingKind::Paper => "paper",
            BindingKind::Note => "note",
            BindingKind::ReadingNote => "readingNote",
        };
        // One SQLite statement keeps bindings and the retained row in the same
        // read snapshot even if an unrelated connection changes metadata.
        let (parent_bound, child_bound, turn, context): (bool, bool, Option<String>, Option<String>) = db.query_row(
            "select exists(select 1 from agent_conversation_bindings where conversation_id=?1 and resource_kind=?3 and resource_id=?4), exists(select 1 from agent_conversation_bindings where conversation_id=?2 and resource_kind=?3 and resource_id=?4), (select turn_id from agent_submissions where conversation_id=?2 and request_id=?5), (select context_snapshot_id from agent_submissions where conversation_id=?2 and request_id=?5)",
            params![parent.to_string(), child.to_string(), kind, binding.resource_id, request.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).map_err(|error| receipts::storage(request, fence, "child-submission-status", &error))?;
        if !parent_bound || !child_bound {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                request,
                fence,
                "child-submission-binding",
                "submission_binding_changed",
            ))));
        }
        let saved = match (turn, context) {
            (None, None) => None,
            (Some(turn), Some(context)) => Some((turn, context)),
            _ => {
                return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    request,
                    fence,
                    "child-submission-status",
                    "submission_identity_invalid",
                ))));
            }
        };
        let record = saved
            .map(|(turn, context)| {
                let invalid = || {
                    SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                        request,
                        fence,
                        "child-submission-status",
                        "submission_identity_invalid",
                    )))
                };
                let turn = Uuid::parse_str(&turn).map_err(|_| invalid())?;
                let context = Uuid::parse_str(&context).map_err(|_| invalid())?;
                if turn.is_nil() || context.is_nil() {
                    return Err(invalid());
                }
                Ok((turn, context))
            })
            .transpose()?;
        Ok(ChildSubmissionInspection {
            epoch: fence.epoch,
            record,
        })
    }
}
