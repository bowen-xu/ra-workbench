//! Exact paper preparation request and its SQL receipt share one owner.
use super::writing_source::SourcePause;
use super::*;
use crate::agent::{reading_submission, repository::submissions};
use crate::{
    agent::reading_context::{PaperContextReader, ReadingNeed, ReplayObservation},
    ai::{paper_reading_source::PaperReadingSource, structured_resource_validation::Flow},
};
use research_agent_core::{
    conversation::CatalogError, protocol::ModelIdentity, submission::SubmissionRecord,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub(crate) const PAPER_PREPARE_FAMILY: &str = "agent.paper.submission.prepare";

#[derive(Clone)]
pub(crate) struct PaperPrepareIntent {
    pub request: reading_submission::PrepareRequest,
    pub expected_epoch: Option<Uuid>,
}
pub(crate) struct PaperPrepareSource {
    pub store: crate::storage::AppStore,
    pub model: (ModelIdentity, Option<String>, Option<String>),
}
pub(crate) enum PaperPrepareStatus {
    Committed {
        record: Box<SubmissionRecord>,
        receipt: Box<SqlImmediateReceipt>,
    },
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl PaperPrepareIntent {
    pub(crate) fn operation(&self) -> Uuid {
        self.request.request_id
    }
}

fn identity(command: &PaperPrepareIntent) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    let request = &command.request;
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.text.len() > 1_048_576
        || command.expected_epoch.is_some_and(|epoch| epoch.is_nil())
        || request
            .retrieval
            .as_ref()
            .is_some_and(|value| value.validate().is_err())
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
    let mut value = serde_json::json!({"schemaVersion": 1, "request": request});
    if let Some(epoch) = command.expected_epoch {
        value["expectedEpoch"] = serde_json::json!(epoch);
    }
    sort(&mut value);
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?;
    let mut hash = Sha256::new();
    hash.update(b"agent.paper.submission.prepare:1:");
    hash.update(bytes);
    Ok(ReceiptIdentity {
        operation_id: command.operation(),
        kind: PAPER_PREPARE_FAMILY,
        version: 1,
        fingerprint: hash.finalize().into(),
    })
}

fn check_epoch(command: &PaperPrepareIntent, fence: ReadFence) -> Result<(), SqlImmediateFailure> {
    if command
        .expected_epoch
        .is_some_and(|epoch| epoch != fence.epoch)
    {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            command.operation(),
            fence,
            "paper-prepare-epoch",
            "data_epoch_changed",
        ))));
    }
    Ok(())
}

fn corrupt(
    command: &PaperPrepareIntent,
    fence: ReadFence,
    cause: &'static str,
) -> SqlImmediateFailure {
    SqlImmediateFailure::Corrupt(Box::new(diagnostic(
        command.operation(),
        fence,
        "paper-prepare",
        cause,
    )))
}
fn map_error(
    command: &PaperPrepareIntent,
    fence: ReadFence,
    error: reading_submission::Error,
) -> SqlImmediateFailure {
    use reading_submission::Error as E;
    let (kind, cause) = match error {
        E::InvalidInput => (0, "paper_preparation_invalid_input"),
        E::Unbound => (0, "paper_preparation_unbound"),
        E::Conflict => (1, "paper_preparation_conflict"),
        E::ModelUnavailable => (2, "paper_preparation_model_unavailable"),
        E::Budget => (0, "paper_preparation_budget"),
        E::Unavailable => (2, "paper_preparation_unavailable"),
        E::OutcomeUnknown(_) => (3, "paper_preparation_nested_unknown"),
        E::Storage(_) => (3, "paper_preparation_nested_storage"),
        E::Source(_) => (3, "paper_preparation_nested_source_failure"),
    };
    let detail = Box::new(diagnostic(
        command.operation(),
        fence,
        "paper-prepare",
        cause,
    ));
    match kind {
        0 => SqlImmediateFailure::Invalid(detail),
        1 => SqlImmediateFailure::Conflict(detail),
        3 => SqlImmediateFailure::Corrupt(detail),
        _ => SqlImmediateFailure::Unavailable(detail),
    }
}
fn record_in(
    db: &Connection,
    command: &PaperPrepareIntent,
    fence: ReadFence,
) -> Result<Option<submissions::ObservedSubmission>, SqlImmediateFailure> {
    use crate::agent::repository::SqlRepositoryError as E;
    submissions::read_sql_observed(db, command.request.conversation_id, command.operation())
        .map_err(|error| match error {
            E::Sql(error) => {
                receipts::storage(command.operation(), fence, "paper-prepare-read", &error)
            }
            E::Domain(_) => corrupt(command, fence, "paper_prepare_record_invalid"),
        })
}
fn preview_failure(
    command: &PaperPrepareIntent,
    fence: ReadFence,
    error: crate::ai::paper_reading_contracts::PaperContextError,
) -> SqlImmediateFailure {
    use crate::ai::paper_reading_contracts::PaperContextError as E;
    match error {
        E::Corrupt => corrupt(command, fence, "paper_prepare_snapshot_invalid"),
        E::Conflict => map_error(command, fence, reading_submission::Error::Conflict),
        E::InvalidInput => map_error(command, fence, reading_submission::Error::InvalidInput),
        E::Budget => map_error(command, fence, reading_submission::Error::Budget),
        E::Unavailable => map_error(command, fence, reading_submission::Error::Unavailable),
    }
}
fn static_failure(flow: Flow<SqlImmediateFailure, ReadingNeed>) -> SqlImmediateFailure {
    match flow {
        Flow::Fail(failure) => failure.error,
        // The static function-value adapter cannot request a Native decision.
        Flow::Need(_) => unreachable!("static paper preparation cannot pause"),
    }
}
fn replay_with_reader(
    reader: &mut dyn PaperContextReader,
    pause: &mut SourcePause<ReadingNeed>,
    db: &Connection,
    command: &PaperPrepareIntent,
    fence: ReadFence,
    receipt: Option<SqlImmediateReceipt>,
) -> Result<Option<SubmissionRecord>, SqlImmediateFailure> {
    let request = &command.request;
    reading_submission::check_binding(db, request.conversation_id, &request.paper_id)
        .map_err(|error| map_error(command, fence, error))?;
    match (receipt, record_in(db, command, fence)?) {
        (None, None) => {
            let partial: bool = db.query_row(
                "select exists(select 1 from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2) or exists(select 1 from agent_submission_images where conversation_id=?1 and request_id=?2)",
                params![request.conversation_id.to_string(), request.request_id.to_string()],
                |row| row.get(0),
            ).map_err(|error| receipts::storage(command.operation(), fence, "paper-prepare-read", &error))?;
            if partial {
                return Err(corrupt(command, fence, "paper_prepare_orphaned_data"));
            }
            Ok(None)
        }
        (None, Some(_)) => Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            command.operation(),
            fence,
            "paper-prepare",
            "legacy_paper_preparation_without_receipt",
        )))),
        (Some(_), None) => Err(corrupt(command, fence, "paper_prepare_record_missing")),
        (Some(receipt), Some(observed)) => {
            let record = &observed.record;
            let scope: Option<(String, String, String, String)> = db.query_row("select turn_id,paper_id,paper_context_snapshot_id,context_hash from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2", params![request.conversation_id.to_string(), request.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|error| receipts::storage(command.operation(), fence, "paper-prepare-scope", &error))?;
            let grants: bool = db
                .query_row(
                    "select exists(select 1 from agent_turn_resource_grants where turn_id=?1)",
                    [record.turn_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    receipts::storage(command.operation(), fence, "paper-prepare-grants", &error)
                })?;
            let manifest =
                submissions::read_image_manifest(db, request.conversation_id, request.request_id)
                    .map_err(|error| match error {
                    research_agent_core::submission::SubmissionError::Unavailable => {
                        map_error(command, fence, reading_submission::Error::Unavailable)
                    }
                    _ => corrupt(command, fence, "paper_prepare_image_manifest_invalid"),
                })?;
            pause.capture(
                reader
                    .observe_replay(ReplayObservation {
                        receipt,
                        submission: observed.clone(),
                        scope: scope.clone(),
                        grants,
                        images: manifest.clone(),
                    })
                    .map_err(|flow| flow.map_error(|error| preview_failure(command, fence, error))),
                fence,
            )?;
            let preview = pause.capture(
                reader
                    .preview(
                        db,
                        &crate::ai::paper_reading_contracts::PaperContextRequest {
                            paper_id: request.paper_id.clone(),
                            snapshot_id: request.paper_context_snapshot_id.clone(),
                            expected_context_hash: request.expected_context_hash.clone(),
                        },
                    )
                    .map_err(|flow| flow.map_error(|error| preview_failure(command, fence, error))),
                fence,
            )?;
            let expected_images =
                crate::ai::paper_context::agent_preview::submission_image_references_from_preview(
                    preview, db,
                )
                .map_err(|error| preview_failure(command, fence, error))?;
            if scope
                != Some((
                    record.turn_id.to_string(),
                    request.paper_id.clone(),
                    request.paper_context_snapshot_id.clone(),
                    request.expected_context_hash.clone(),
                ))
                || !grants
                || record.input.text != request.text
                || record.input.user_input != request.user_input
                || manifest != expected_images
                || reading_submission::validate_record_with_images(
                    record,
                    &request.paper_id,
                    &request.paper_context_snapshot_id,
                    !manifest.is_empty(),
                )
                .is_err()
            {
                return Err(corrupt(command, fence, "paper_prepare_result_invalid"));
            }
            Ok(Some(observed.record))
        }
    }
}

impl Database {
    /// A missing receipt is not evidence of failure while a prepare worker may still run.
    pub(crate) fn paper_prepare_status(
        &self,
        command: &PaperPrepareIntent,
        mut reading_source: PaperReadingSource,
    ) -> Result<PaperPrepareStatus, SqlImmediateFailure> {
        self.paper_prepare_status_with_reader(command, &mut reading_source)
            .map_err(static_failure)
    }
    pub(crate) fn paper_prepare_status_with_reader(
        &self,
        command: &PaperPrepareIntent,
        reader: &mut dyn PaperContextReader,
    ) -> Result<PaperPrepareStatus, Flow<SqlImmediateFailure, ReadingNeed>> {
        let mut pause = SourcePause::paper(command.operation(), "paper-prepare-status-pause");
        let result = self.paper_prepare_status_in(command, reader, &mut pause);
        pause.finish(result)
    }
    fn paper_prepare_status_in(
        &self,
        command: &PaperPrepareIntent,
        reader: &mut dyn PaperContextReader,
        pause: &mut SourcePause<ReadingNeed>,
    ) -> Result<PaperPrepareStatus, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.operation()))?;
        let guard = owner
            .inspection_guard()
            .map_err(|error| admission_error(command.operation(), error, None))?;
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        check_epoch(command, cached)?;
        let db = self
            .raw
            .lock()
            .map_err(|_| corrupt(command, cached, "sql_mutex_poisoned"))?;
        let unconfirmed = |fence, cause| {
            PaperPrepareStatus::Unconfirmed(Box::new(diagnostic(
                command.operation(),
                fence,
                "paper-prepare-status",
                cause,
            )))
        };
        if !db.is_autocommit() {
            return Ok(unconfirmed(cached, "sql_transaction_open"));
        }
        let fence =
            receipts::status_fence(&db, command.operation(), cached, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, command.operation(), fence, "paper-prepare-status")?;
        let receipt = receipts::lookup(&db, &identity, fence, "paper-prepare-status")?;
        if let Some(receipt) = receipt {
            let record =
                replay_with_reader(reader, pause, &db, command, fence, Some(receipt.clone()))?
                    .ok_or_else(|| corrupt(command, fence, "paper_prepare_record_missing"))?;
            return Ok(PaperPrepareStatus::Committed {
                record: Box::new(record),
                receipt: Box::new(receipt),
            });
        }
        Ok(unconfirmed(fence, "paper_prepare_worker_unconfirmed"))
    }
    /// Replay an exact proof before crossing the model or source boundary.
    pub(crate) fn replay_paper_prepare_with_receipt(
        &self,
        command: &PaperPrepareIntent,
        mut reading_source: PaperReadingSource,
    ) -> Result<Option<(SubmissionRecord, SqlImmediateReceipt)>, SqlImmediateFailure> {
        self.replay_paper_prepare_with_reader(command, &mut reading_source)
            .map_err(static_failure)
    }
    pub(crate) fn replay_paper_prepare_with_reader(
        &self,
        command: &PaperPrepareIntent,
        reader: &mut dyn PaperContextReader,
    ) -> Result<
        Option<(SubmissionRecord, SqlImmediateReceipt)>,
        Flow<SqlImmediateFailure, ReadingNeed>,
    > {
        let mut pause = SourcePause::paper(command.operation(), "paper-prepare-replay-pause");
        let result = self.replay_paper_prepare_in(command, reader, &mut pause);
        pause.finish(result)
    }
    fn replay_paper_prepare_in(
        &self,
        command: &PaperPrepareIntent,
        reader: &mut dyn PaperContextReader,
        pause: &mut SourcePause<ReadingNeed>,
    ) -> Result<Option<(SubmissionRecord, SqlImmediateReceipt)>, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.operation()))?;
        let mut guard = owner
            .materialize_guard()
            .map_err(|error| admission_error(command.operation(), error, None))?;
        let fence = guard.fence();
        check_epoch(command, fence)?;
        let db = self.raw.lock().map_err(|_| {
            guard.block();
            corrupt(command, fence, "sql_mutex_poisoned")
        })?;
        if !db.is_autocommit() {
            guard.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.operation(),
                fence,
                "paper-prepare-replay",
                "sql_transaction_already_open",
            ))));
        }
        let result = (|| {
            receipts::check_state(&db, command.operation(), fence, "paper-prepare-replay")?;
            receipts::check_cross_kind(&db, command.operation(), fence, "paper-prepare-replay")?;
            let receipt = receipts::lookup(&db, &identity, fence, "paper-prepare-replay")?;
            let record = replay_with_reader(reader, pause, &db, command, fence, receipt.clone())?;
            Ok(record.map(|record| (record, receipt.expect("replay requires receipt"))))
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            guard.block();
        }
        result
    }
    pub(crate) fn prepare_paper_with_receipt(
        &self,
        command: &PaperPrepareIntent,
        source: PaperPrepareSource,
    ) -> Result<(SubmissionRecord, SqlImmediateReceipt), SqlImmediateFailure> {
        self.prepare_paper_owned(
            command,
            source,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn prepare_paper_owned(
        &self,
        command: &PaperPrepareIntent,
        source: PaperPrepareSource,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<(SubmissionRecord, SqlImmediateReceipt), SqlImmediateFailure> {
        self.prepare_paper_attempt(
            command,
            source,
            None,
            #[cfg(test)]
            fault,
        )
        .map_err(static_failure)
    }
    pub(crate) fn prepare_paper_with_reader(
        &self,
        command: &PaperPrepareIntent,
        source: PaperPrepareSource,
        reader: &mut dyn PaperContextReader,
    ) -> Result<(SubmissionRecord, SqlImmediateReceipt), Flow<SqlImmediateFailure, ReadingNeed>>
    {
        self.prepare_paper_attempt(
            command,
            source,
            Some(reader),
            #[cfg(test)]
            super::Fault::None,
        )
    }
    fn prepare_paper_attempt(
        &self,
        command: &PaperPrepareIntent,
        source: PaperPrepareSource,
        reader: Option<&mut dyn PaperContextReader>,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<(SubmissionRecord, SqlImmediateReceipt), Flow<SqlImmediateFailure, ReadingNeed>>
    {
        let identity = identity(command).map_err(Flow::original)?;
        let mut result = None;
        let mut pause = SourcePause::paper(command.operation(), "paper-prepare-source-pause");
        let executed = self.execute_sql(
            command.operation(),
            |tx, fence| {
                check_epoch(command, fence)?;
                receipts::check_state(tx, command.operation(), fence, "paper-prepare")?;
                receipts::check_cross_kind(tx, command.operation(), fence, "paper-prepare")?;
                let receipt = receipts::lookup(tx, &identity, fence, "paper-prepare")?;
                let mut static_reader;
                let reader = match reader {
                    Some(reader) => reader,
                    None => {
                        static_reader = source.store.paper_reading_source().ok_or_else(|| {
                            map_error(command, fence, reading_submission::Error::Unavailable)
                        })?;
                        &mut static_reader
                    }
                };
                if let Some(record) =
                    replay_with_reader(reader, &mut pause, tx, command, fence, receipt.clone())?
                {
                    result = Some(record);
                    return Ok(receipt.expect("replay requires receipt"));
                }
                let record = pause.capture(
                    reading_submission::materialize_with_reader(
                        tx,
                        &source.store,
                        command.request.clone(),
                        source.model,
                        reader,
                    )
                    .map_err(|flow| flow.map_error(|error| map_error(command, fence, error))),
                    fence,
                )?;
                result = Some(record);
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        );
        let completed = executed.and_then(|receipt| {
            Ok((
                result.ok_or(SqlImmediateFailure::Domain(CatalogError::Corrupt))?,
                receipt,
            ))
        });
        pause.finish(completed)
    }
}
