//! One exact independent-context preparation and its receipt share a SQL transaction.
use super::*;
use crate::agent::{context_session, repository::submissions};
use research_agent_core::{conversation::CatalogError, protocol::ModelIdentity};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

#[path = "context_prepare/mutation.rs"]
mod mutation;
pub(crate) use mutation::ContextPrepareScopeFailure;

const FAMILY: &str = "agent.context.submission.prepare";

#[derive(Clone)]
pub(crate) struct ContextPrepareIntent {
    pub request: context_session::PrepareRequest,
    pub expected_epoch: Option<Uuid>,
}
#[derive(Clone)]
pub(crate) struct ContextPrepareSource {
    pub store: crate::storage::AppStore,
    pub model: (ModelIdentity, Option<String>, Option<String>),
}
pub(crate) enum ContextPrepareStatus {
    Committed {
        preview: Box<context_session::Preview>,
        receipt: Box<SqlImmediateReceipt>,
    },
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
fn identity(command: &ContextPrepareIntent) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    let request = &command.request;
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.text.trim().is_empty()
        || request.text.len() > 1_048_576
        || request.resources.len() > 32
        || request
            .retrieval
            .as_ref()
            .is_some_and(|selection| selection.validate().is_err())
        || command.expected_epoch.is_some_and(|epoch| epoch.is_nil())
    {
        return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput));
    }
    fn sort(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.values_mut().for_each(sort);
                map.sort_keys();
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(sort),
            _ => {}
        }
    }
    let mut value = serde_json::json!({"schemaVersion":1,"request":request});
    if let Some(epoch) = command.expected_epoch {
        value["expectedEpoch"] = serde_json::json!(epoch);
    }
    sort(&mut value);
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?;
    let mut hash = Sha256::new();
    hash.update(b"agent.context.submission.prepare:1:");
    hash.update(bytes);
    Ok(ReceiptIdentity {
        operation_id: request.request_id,
        kind: FAMILY,
        version: 1,
        fingerprint: hash.finalize().into(),
    })
}
fn error(
    command: &ContextPrepareIntent,
    fence: ReadFence,
    cause: context_session::Error,
) -> SqlImmediateFailure {
    use context_session::Error as E;
    let (class, reason) = match cause {
        E::InvalidInput => (0, "context_prepare_invalid_input"),
        E::ConversationUnavailable => (0, "context_prepare_conversation_unavailable"),
        E::SourceUnavailable => (2, "context_prepare_source_unavailable"),
        E::NoteTextViewUnstable { .. } => (2, "MODEL_NOTE_TEXT_VIEW_UNSTABLE"),
        E::PaperNotIndexed => (2, "context_prepare_paper_not_indexed"),
        E::PaperNotParsed => (2, "context_prepare_paper_not_parsed"),
        E::PaperParseFailed => (2, "context_prepare_paper_parse_failed"),
        E::Budget => (0, "context_prepare_budget"),
        E::ModelUnavailable => (2, "context_prepare_model_unavailable"),
        E::ConfirmationMismatch => (1, "context_prepare_confirmation_mismatch"),
        E::Unavailable | E::NativeCatalogUnavailable(_) => (2, "context_prepare_unavailable"),
        E::QueueAdmissionUnknown {
            request_id,
            phase,
            safe_cause,
            sqlite_code,
            sqlite_extended_code,
            ..
        } => {
            let mut detail =
                SqlImmediateDiagnostic::for_operation(request_id, None, phase, safe_cause);
            detail.sqlite_code = sqlite_code;
            detail.sqlite_extended_code = sqlite_extended_code;
            return SqlImmediateFailure::Unknown(Box::new(detail));
        }
        E::TaskObservationUnavailable => (2, "context_prepare_task_observation_unavailable"),
    };
    let detail = Box::new(diagnostic(
        command.request.request_id,
        fence,
        "context-prepare",
        reason,
    ));
    match class {
        0 => SqlImmediateFailure::Invalid(detail),
        1 => SqlImmediateFailure::Conflict(detail),
        3 => SqlImmediateFailure::Corrupt(detail),
        _ => SqlImmediateFailure::Unavailable(detail),
    }
}
fn check_epoch(
    command: &ContextPrepareIntent,
    fence: ReadFence,
) -> Result<(), SqlImmediateFailure> {
    if command
        .expected_epoch
        .is_some_and(|epoch| epoch != fence.epoch)
    {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            command.request.request_id,
            fence,
            "context-prepare-epoch",
            "data_epoch_changed",
        ))));
    }
    Ok(())
}
fn replay_in(
    db: &Connection,
    command: &ContextPrepareIntent,
    fence: ReadFence,
    receipt: Option<&(SqlImmediateReceipt, String)>,
) -> Result<Option<context_session::Preview>, SqlImmediateFailure> {
    let request = &command.request;
    context_session::check_conversation(db, request.conversation_id)
        .map_err(|cause| error(command, fence, cause))?;
    let record = submissions::read_sql(db, request.conversation_id, request.request_id).map_err(
        |cause| match cause {
            crate::agent::repository::SqlRepositoryError::Sql(cause) => {
                receipts::storage(request.request_id, fence, "context-prepare-read", &cause)
            }
            crate::agent::repository::SqlRepositoryError::Domain(_) => {
                SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    request.request_id,
                    fence,
                    "context-prepare-read",
                    "context_prepare_record_invalid",
                )))
            }
        },
    )?;
    let Some((_, encoded)) = receipt else {
        if record.is_some() {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                request.request_id,
                fence,
                "context-prepare-replay",
                "legacy_context_preparation_without_receipt",
            ))));
        }
        return Ok(None);
    };
    let record = record.ok_or_else(|| {
        SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            request.request_id,
            fence,
            "context-prepare-replay",
            "context_prepare_record_missing",
        )))
    })?;
    if record.input.text != request.text {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            request.request_id,
            fence,
            "context-prepare-replay",
            "context_prepare_result_invalid",
        ))));
    }
    let link: Option<(String, u64, String)> = db.query_row("select input_id,expected_revision,result_json from agent_input_queue_previews where conversation_id=?1 and request_id=?2", params![request.conversation_id.to_string(),request.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|cause| receipts::storage(request.request_id, fence, "context-prepare-queue", &cause))?;
    let mut preview = context_session::Preview::from(record);
    match (&request.queue, link) {
        (None, None) => {}
        (Some(expected), Some((input, revision, snapshot)))
            if input == expected.input_id.to_string() && revision == expected.expected_revision =>
        {
            let frozen = crate::agent::repository::input_queue::decode_prepared_snapshot(
                &snapshot,
                request.conversation_id,
            )
            .map_err(|_| {
                SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    request.request_id,
                    fence,
                    "context-prepare-queue",
                    "context_prepare_queue_snapshot_invalid",
                )))
            })?;
            let next_revision = revision
                .checked_add(1)
                .ok_or(SqlImmediateFailure::Domain(CatalogError::InvalidInput))?;
            let original = frozen
                .inputs()
                .iter()
                .find(|item| item.id == expected.input_id);
            if frozen.revision() != next_revision
                || frozen.preparing() != Some(expected.input_id)
                || original.is_none_or(|item| {
                    item.text != request.text || item.user_input != request.user_input
                })
            {
                return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    request.request_id,
                    fence,
                    "context-prepare-queue",
                    "context_prepare_queue_snapshot_mismatch",
                ))));
            }
            let origin: Option<String> = db.query_row("select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2", params![request.conversation_id.to_string(),request.request_id.to_string()], |row| row.get(0)).optional().map_err(|cause| receipts::storage(request.request_id, fence, "context-prepare-queue", &cause))?;
            if origin.as_deref() != Some(input.as_str()) {
                return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    request.request_id,
                    fence,
                    "context-prepare-queue",
                    "context_prepare_queue_origin_mismatch",
                ))));
            }
            preview.queue = Some(crate::agent::queue_contract::Confirmation {
                input_id: expected.input_id,
                expected_revision: next_revision,
            });
        }
        _ => {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                request.request_id,
                fence,
                "context-prepare-replay",
                "context_prepare_queue_not_retained",
            ))));
        }
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FrozenResult {
        sha256: String,
    }
    let frozen: FrozenResult = serde_json::from_str(encoded).map_err(|_| {
        SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            request.request_id,
            fence,
            "context-prepare-replay",
            "context_prepare_result_proof_invalid",
        )))
    })?;
    if frozen.sha256
        != preview_hash(&preview).map_err(|_| {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                request.request_id,
                fence,
                "context-prepare-replay",
                "context_prepare_result_encoding_invalid",
            )))
        })?
    {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            request.request_id,
            fence,
            "context-prepare-replay",
            "context_prepare_result_proof_mismatch",
        ))));
    }
    Ok(Some(preview))
}
fn preview_hash(preview: &context_session::Preview) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(preview)?)
    ))
}
impl Database {
    /// Absence of a receipt is not proof that an asynchronous worker stopped.
    pub(crate) fn context_prepare_status(
        &self,
        command: &ContextPrepareIntent,
    ) -> Result<ContextPrepareStatus, SqlImmediateFailure> {
        let identity = identity(command)?;
        let operation = command.request.request_id;
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(operation))?;
        let guard = owner
            .inspection_guard()
            .map_err(|cause| admission_error(operation, cause, None))?;
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        check_epoch(command, cached)?;
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation,
                cached,
                "context-prepare-status",
                "sql_mutex_poisoned",
            )))
        })?;
        let unconfirmed = |fence, cause| {
            ContextPrepareStatus::Unconfirmed(Box::new(diagnostic(
                operation,
                fence,
                "context-prepare-status",
                cause,
            )))
        };
        if !db.is_autocommit() {
            return Ok(unconfirmed(cached, "sql_transaction_open"));
        }
        let fence = receipts::status_fence(&db, operation, cached, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, operation, fence, "context-prepare-status")?;
        let receipt =
            receipts::lookup_with_result(&db, &identity, fence, "context-prepare-status", false)?;
        if let Some(receipt) = receipt {
            let preview = replay_in(&db, command, fence, Some(&receipt))?.ok_or_else(|| {
                SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    operation,
                    fence,
                    "context-prepare-status",
                    "context_prepare_record_missing",
                )))
            })?;
            return Ok(ContextPrepareStatus::Committed {
                preview: Box::new(preview),
                receipt: Box::new(receipt.0),
            });
        }
        Ok(unconfirmed(fence, "context_prepare_worker_unconfirmed"))
    }
    pub(crate) fn replay_context_prepare(
        &self,
        command: &ContextPrepareIntent,
    ) -> Result<Option<(context_session::Preview, SqlImmediateReceipt)>, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.request.request_id))?;
        let mut guard = owner
            .materialize_guard()
            .map_err(|cause| admission_error(command.request.request_id, cause, None))?;
        let fence = guard.fence();
        check_epoch(command, fence)?;
        let db = self.raw.lock().map_err(|_| {
            guard.block();
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                command.request.request_id,
                fence,
                "context-prepare-replay",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            guard.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request.request_id,
                fence,
                "context-prepare-replay",
                "sql_transaction_already_open",
            ))));
        }
        let result = (|| {
            receipts::check_state(
                &db,
                command.request.request_id,
                fence,
                "context-prepare-replay",
            )?;
            receipts::check_cross_kind(
                &db,
                command.request.request_id,
                fence,
                "context-prepare-replay",
            )?;
            let receipt = receipts::lookup_with_result(
                &db,
                &identity,
                fence,
                "context-prepare-replay",
                false,
            )?;
            Ok(replay_in(&db, command, fence, receipt.as_ref())?
                .map(|value| (value, receipt.expect("receipt required").0)))
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            guard.block();
        }
        result
    }
}

#[cfg(test)]
#[path = "context_prepare_tests.rs"]
mod tests;
