//! Shared Agent admission for one explicitly selected immutable paper snapshot.
//! Preparing or restoring a receipt does not start inference or read image bytes.
use super::{
    model::{AgentHost, AgentModelResolver, ModelSelection},
    repository::submissions::read,
    retrieval_context::RetrievalSelection,
};
use crate::{agent::config::ConfigOverride, ai::paper_context::agent_preview, storage::AppStore};
use research_agent_core::submission::{
    ContextSourceKind, SubmissionError, SubmissionInput, SubmissionRecord,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
#[path = "reading_submission/materialize.rs"]
mod materialize;
#[cfg(test)]
pub(crate) use materialize::materialize_in;
pub(crate) use materialize::materialize_with_reader;
#[path = "reading_submission/prepare_status.rs"]
pub(crate) mod prepare_status;
#[path = "reading_submission/source_failure.rs"]
mod source_failure;
pub use source_failure::SourceFailure;
#[path = "reading_submission/transaction.rs"]
pub(super) mod transaction;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
    pub expected_context_hash: String,
    #[serde(default)]
    pub retrieval: Option<RetrievalSelection>,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub model: Option<ModelSelection>,
    #[serde(default)]
    pub session_config: Option<ConfigOverride>,
    #[serde(default)]
    pub turn_config: Option<ConfigOverride>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid paper submission")]
    InvalidInput,
    #[error("paper conversation binding unavailable")]
    Unbound,
    #[error("paper snapshot changed; review the selected sources again")]
    Conflict,
    #[error("paper model configuration unavailable")]
    ModelUnavailable,
    #[error("paper submission source exceeds its budget")]
    Budget,
    #[error("paper submission unavailable")]
    Unavailable,
    #[error("paper submission outcome unknown")]
    OutcomeUnknown(transaction::Unknown),
    #[error(transparent)]
    Source(Box<SourceFailure>),
    #[error("paper submission storage failure")]
    Storage(Box<StorageFailure>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct StorageFailure {
    pub code: &'static str,
    pub operation_id: Uuid,
    pub epoch: Option<Uuid>,
    pub sql_fence: Option<String>,
    pub phase: &'static str,
    pub safe_cause: &'static str,
    pub sqlite_code: Option<String>,
    pub sqlite_extended_code: Option<i32>,
}

impl Serialize for StorageFailure {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Diagnostic<'a> {
            operation_id: Uuid,
            epoch: Option<Uuid>,
            sql_fence: &'a Option<String>,
            phase: &'static str,
            safe_cause: &'static str,
            sqlite_code: &'a Option<String>,
            sqlite_extended_code: Option<i32>,
        }
        let mut value = serializer.serialize_struct("PaperStorageFailure", 2)?;
        value.serialize_field("code", self.code)?;
        value.serialize_field(
            "reason",
            &Diagnostic {
                operation_id: self.operation_id,
                epoch: self.epoch,
                sql_fence: &self.sql_fence,
                phase: self.phase,
                safe_cause: self.safe_cause,
                sqlite_code: &self.sqlite_code,
                sqlite_extended_code: self.sqlite_extended_code,
            },
        )?;
        value.end()
    }
}

impl Error {
    pub(crate) fn worker_unsettled(request: &PrepareRequest) -> Self {
        Self::OutcomeUnknown(transaction::Unknown {
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            phase: "paper-prepare-worker",
            safe_cause: "paper_submission_worker_unsettled",
            sqlite_code: None,
            sqlite_extended_code: None,
        })
    }
    pub(super) fn code(&self) -> &str {
        match self {
            Self::InvalidInput => "invalidInput",
            Self::Unbound => "unbound",
            Self::Conflict => "conflict",
            Self::ModelUnavailable => "modelUnavailable",
            Self::Budget => "budget",
            Self::Unavailable => "unavailable",
            Self::OutcomeUnknown(_) => "outcomeUnknown",
            Self::Storage(_) => "storage",
            Self::Source(source) => source.code(),
        }
    }
}
impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            Self::InvalidInput => serializer.serialize_str("invalidInput"),
            Self::Unbound => serializer.serialize_str("unbound"),
            Self::Conflict => serializer.serialize_str("conflict"),
            Self::ModelUnavailable => serializer.serialize_str("modelUnavailable"),
            Self::Budget => serializer.serialize_str("budget"),
            Self::Unavailable => serializer.serialize_str("unavailable"),
            Self::OutcomeUnknown(detail) => {
                serializer.serialize_newtype_variant("Error", 6, "outcomeUnknown", detail)
            }
            Self::Source(detail) => detail.serialize(serializer),
            Self::Storage(detail) => {
                let mut value = serializer.serialize_struct("PaperPreparationError", 2)?;
                value.serialize_field("code", "storage")?;
                value.serialize_field("reason", detail)?;
                value.end()
            }
        }
    }
}

pub(crate) fn storage_error(error: crate::data_interface::SqlImmediateFailure) -> Error {
    use crate::data_interface::SqlImmediateFailure as F;
    use research_agent_core::conversation::CatalogError;
    let (code, detail) = match error {
        F::Domain(CatalogError::InvalidInput) => return Error::InvalidInput,
        F::Domain(CatalogError::NotFound) => return Error::Unbound,
        F::Domain(CatalogError::Conflict) => return Error::Conflict,
        F::Domain(_) => return Error::Unavailable,
        F::Invalid(detail) => ("DATA_SQL_INVALID", detail),
        F::Unmanaged(detail) => ("DATA_SQL_UNMANAGED", detail),
        F::Conflict(detail) => ("DATA_SQL_OPERATION_CONFLICT", detail),
        F::Unknown(detail) => ("DATA_SQL_COMMIT_UNKNOWN", detail),
        F::RecoveryRequired(detail) => ("DATA_SQL_RECOVERY_REQUIRED", detail),
        F::Unavailable(detail) => ("DATA_SQL_UNAVAILABLE", detail),
        F::Corrupt(detail) => ("DATA_SQL_CORRUPT", detail),
    };
    Error::Storage(Box::new(StorageFailure {
        code,
        operation_id: detail.operation_id,
        epoch: detail.epoch,
        sql_fence: detail.sql_fence,
        phase: detail.phase,
        safe_cause: detail.safe_cause,
        sqlite_code: detail.sqlite_code,
        sqlite_extended_code: detail.sqlite_extended_code,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: SubmissionInput,
}

impl Preview {
    pub(crate) fn from_record(record: SubmissionRecord, receipt: &Receipt) -> Self {
        Self {
            schema_version: 1,
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            paper_id: receipt.paper_id.clone(),
            paper_context_snapshot_id: receipt.paper_context_snapshot_id.clone(),
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input: record.input,
        }
    }
}

#[tauri::command]
pub(crate) async fn agent_prepare_paper_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    let receipt = Receipt {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        paper_id: request.paper_id.clone(),
        paper_context_snapshot_id: request.paper_context_snapshot_id.clone(),
    };
    let record = prepare(&store, &models, request).await?;
    Ok(Preview::from_record(record, &receipt))
}

#[tauri::command]
pub(crate) async fn agent_prepare_paper_submission_proven(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<prepare_status::Response, Error> {
    prepare_proven(store.inner(), models.inner(), request, expected_epoch).await
}

pub(crate) async fn prepare_proven(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<prepare_status::Response, Error> {
    let (record, receipt) =
        prepare_with_epoch(store, models, request.clone(), expected_epoch).await?;
    let receipt = receipt.ok_or(Error::Unavailable)?;
    Ok(prepare_status::committed(request, record, receipt))
}

#[tauri::command]
pub(crate) async fn agent_restore_paper_submission(
    store: State<'_, AppStore>,
    request: Receipt,
) -> Result<Preview, Error> {
    let record = restore(&store, &request).await?;
    Ok(Preview::from_record(record, &request))
}

pub(crate) async fn prepare(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<SubmissionRecord, Error> {
    prepare_with_receipt(store, models, request)
        .await
        .map(|(record, _)| record)
}

pub(crate) async fn prepare_with_receipt(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<
    (
        SubmissionRecord,
        Option<crate::data_interface::SqlImmediateReceipt>,
    ),
    Error,
> {
    prepare_with_epoch(store, models, request, None).await
}

async fn prepare_with_epoch(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
    expected_epoch: Option<Uuid>,
) -> Result<
    (
        SubmissionRecord,
        Option<crate::data_interface::SqlImmediateReceipt>,
    ),
    Error,
> {
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.text.len() > 1_048_576
        || request
            .retrieval
            .as_ref()
            .is_some_and(|selection| selection.validate().is_err())
    {
        return Err(Error::InvalidInput);
    }
    let check = store.clone();
    let conversation = request.conversation_id;
    let paper = request.paper_id.clone();
    tokio::task::spawn_blocking(move || {
        let db = check.connection().map_err(|_| Error::Unavailable)?;
        check_binding(&db, conversation, &paper)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let intent = crate::data_interface::PaperPrepareIntent {
        request: request.clone(),
        expected_epoch,
    };
    let database = store.shared_agent_connection();
    #[cfg(test)]
    let managed = database.logged();
    #[cfg(not(test))]
    let managed = true;
    if expected_epoch.is_some() && !managed {
        return Err(Error::Unavailable);
    }
    if managed {
        let replayed = if let Some(binding) = store.native_note_trace() {
            binding
                .service
                .replay_paper_submission(store.clone(), binding.package_key.clone(), intent.clone())
                .await?
        } else {
            let replay = intent.clone();
            let db = database.clone();
            let reading_source = store
                .require_paper_reading_source()
                .map_err(|_| Error::Unavailable)?;
            tokio::task::spawn_blocking(move || {
                db.replay_paper_prepare_with_receipt(&replay, reading_source)
            })
            .await
            .map_err(|_| Error::Unavailable)?
            .map_err(storage_error)?
        };
        if let Some((record, receipt)) = replayed {
            return Ok((record, Some(receipt)));
        }
    }
    let model = models
        .prepare(AgentHost::Reading, request.model.clone().map(Into::into))
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let identity = model.identity().clone();
    let reasoning_effort = model.reasoning_effort().map(str::to_owned);
    let service_tier = model.service_tier().map(str::to_owned);
    drop(model);
    let store = store.clone();
    #[cfg(test)]
    if !managed {
        return tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Unavailable)?;
            transaction::prepare(&mut db, request.conversation_id, request.request_id, |tx| {
                materialize_in(
                    tx,
                    &store,
                    request,
                    (identity, reasoning_effort, service_tier),
                )
            })
        })
        .await
        .map_err(|_| Error::Unavailable)?
        .map(|record| (record, None));
    }
    if let Some(binding) = store.native_note_trace() {
        return binding
            .service
            .prepare_paper_submission(
                store.clone(),
                binding.package_key.clone(),
                intent,
                (identity, reasoning_effort, service_tier),
            )
            .await
            .map(|(record, receipt)| (record, Some(receipt)));
    }
    tokio::task::spawn_blocking(move || {
        database
            .prepare_paper_with_receipt(
                &intent,
                crate::data_interface::PaperPrepareSource {
                    store,
                    model: (identity, reasoning_effort, service_tier),
                },
            )
            .map(|(record, receipt)| (record, Some(receipt)))
            .map_err(storage_error)
    })
    .await
    .map_err(|_| {
        Error::Storage(Box::new(StorageFailure {
            code: "DATA_SQL_COMMIT_UNKNOWN",
            operation_id: request.request_id,
            epoch: None,
            sql_fence: None,
            phase: "paper-prepare-worker",
            safe_cause: "worker_join_failed",
            sqlite_code: None,
            sqlite_extended_code: None,
        }))
    })?
}

pub(crate) async fn restore(
    store: &AppStore,
    receipt: &Receipt,
) -> Result<SubmissionRecord, Error> {
    if receipt.conversation_id.is_nil()
        || receipt.request_id.is_nil()
        || receipt.paper_id.trim().is_empty()
        || receipt.paper_context_snapshot_id.trim().is_empty()
    {
        return Err(Error::InvalidInput);
    }
    let store = store.clone();
    let receipt = receipt.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        check_binding(&tx, receipt.conversation_id, &receipt.paper_id)?;
        let record = read(&tx, receipt.conversation_id, receipt.request_id)
            .map_err(submission_error)?
            .ok_or(Error::Unavailable)?;
        let images = super::repository::submissions::read_image_manifest(
            &tx, receipt.conversation_id, receipt.request_id,
        ).map_err(submission_error)?;
        if record.input.context.is_empty() {
            let scope: Option<(String, String, String)> = tx.query_row(
                "select turn_id,paper_id,paper_context_snapshot_id from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
                params![receipt.conversation_id.to_string(), receipt.request_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional().map_err(|_| Error::Unavailable)?;
            if scope != Some((record.turn_id.to_string(), receipt.paper_id.clone(), receipt.paper_context_snapshot_id.clone())) {
                return Err(Error::Unbound);
            }
        }
        validate_record_with_images(
            &record,
            &receipt.paper_id,
            &receipt.paper_context_snapshot_id,
            !images.is_empty(),
        )?;
        Ok(record)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(crate) fn check_binding(db: &Connection, conversation: Uuid, paper: &str) -> Result<(), Error> {
    let allowed: bool = db
        .query_row(
            "select exists(select 1 from agent_conversation_bindings b
             join agent_conversations c on c.id=b.conversation_id
             join bibliographic_items p on p.id=b.resource_id
             where b.conversation_id=?1 and b.resource_kind='paper' and b.resource_id=?2
             and c.archived=0 and p.trashed_at is null)",
            params![conversation.to_string(), paper],
            |row| row.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    allowed.then_some(()).ok_or(Error::Unbound)
}

#[cfg(test)]
fn validate_record(record: &SubmissionRecord, paper: &str, snapshot: &str) -> Result<(), Error> {
    validate_record_with_images(record, paper, snapshot, false)
}

pub(crate) fn validate_record_with_images(
    record: &SubmissionRecord,
    paper: &str,
    snapshot: &str,
    has_images: bool,
) -> Result<(), Error> {
    crate::agent::skills::application::validate(&record.input).map_err(|_| Error::Unavailable)?;
    super::config::profile_snapshot::validate(
        &record.input,
        super::config::profiles::ProfileKind::Reading,
        &format!("paper:{paper}"),
    )
    .map_err(|_| Error::Unavailable)?;
    if record.input.context.is_empty() && !has_images {
        return Err(Error::Unbound);
    }
    for source in &record.input.context {
        let anchor: serde_json::Value =
            serde_json::from_str(source.anchor.as_deref().ok_or(Error::Unavailable)?)
                .map_err(|_| Error::Unavailable)?;
        match &source.kind {
            ContextSourceKind::Paper
                if source.resource_id == paper
                    && anchor.get("paperId").and_then(serde_json::Value::as_str) == Some(paper)
                    && anchor.get("snapshotId").and_then(serde_json::Value::as_str)
                        == Some(snapshot) => {}
            ContextSourceKind::Retrieval
                if source.resource_id == paper
                    && anchor.get("paperId").and_then(serde_json::Value::as_str) == Some(paper)
                    && anchor.get("snapshotId").and_then(serde_json::Value::as_str)
                        == Some(snapshot)
                    && anchor
                        .get("traceId")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|trace| source.source_id == format!("retrieval:{trace}")) => {}
            _ => return Err(Error::Unbound),
        }
    }
    Ok(())
}

pub(crate) fn preview_error(error: agent_preview::Error) -> Error {
    match error {
        agent_preview::Error::InvalidInput => Error::InvalidInput,
        agent_preview::Error::Conflict => Error::Conflict,
        agent_preview::Error::Budget => Error::Budget,
        agent_preview::Error::Unavailable | agent_preview::Error::Corrupt => Error::Unavailable,
    }
}

fn submission_error(error: SubmissionError) -> Error {
    match error {
        SubmissionError::InvalidInput => Error::InvalidInput,
        SubmissionError::Conflict => Error::Conflict,
        SubmissionError::NotFound | SubmissionError::Unavailable | SubmissionError::Corrupt => {
            Error::Unavailable
        }
    }
}

#[cfg(test)]
#[path = "reading_submission/tests.rs"]
mod tests;
