//! Prepare a native writing submission; no model request or tool execution here.
#[cfg(all(debug_assertions, feature = "development-data"))]
#[path = "writing_submission/native_acceptance_transport.rs"]
mod native_acceptance_transport;
use super::{
    model::{
        AgentHost, AgentModelPreparationError, AgentModelResolver, AgentModelSelection,
        PreparedAgentModel,
    },
    repository::submissions::read,
    retrieval_context::RetrievalSelection,
};
use crate::{
    agent::writing_context::{WritingContextError, WritingContextRequest},
    storage::AppStore,
};
use research_agent_core::config::InstructionConfigPatch;
use research_agent_core::submission::{SubmissionError, SubmissionInput, SubmissionRecord};
use rusqlite::{Connection, TransactionBehavior, params};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct WritingSubmissionReceipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
}

#[derive(Clone)]
pub struct WritingSubmissionRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub context: WritingContextRequest,
    pub resources: Vec<super::context_session::sources::Resource>,
    pub retrieval: Option<RetrievalSelection>,
    pub model: Option<AgentModelSelection>,
    pub session_config: Option<InstructionConfigPatch>,
    pub turn_config: Option<InstructionConfigPatch>,
}
/// Native-only; the prepared model contains credentials and must never be serialized.
pub struct PreparedWritingSubmission {
    pub record: SubmissionRecord,
    pub model: PreparedAgentModel,
}
#[path = "writing_submission/preparation.rs"]
mod preparation;
#[path = "writing_submission/prepare_owned.rs"]
mod prepare_owned;
pub(crate) use prepare_owned::worker_failure as prepare_worker_failure;
#[path = "writing_submission/start_observation.rs"]
mod start_observation;
pub(crate) use preparation::FrozenWritingModel;
#[cfg(test)]
pub(crate) use preparation::materialize_submission_in;
pub(crate) use preparation::materialize_submission_with_context;
#[path = "writing_submission/compaction.rs"]
mod compaction;
pub(crate) use compaction::{PreparedWritingCompaction, read_in as read_compaction_revision_in};
pub(crate) use compaction::{
    check_prepare_in as check_compaction_prepare_in, freeze_in as freeze_compaction_in,
};
#[path = "writing_submission/diagnostics.rs"]
mod diagnostics;
pub use diagnostics::WritingFailure;
pub(crate) use diagnostics::WritingSourceDiagnostic;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WritingStorageError(
    Box<crate::data_interface::SqlImmediateFailure>,
    Option<WritingSourceDiagnostic>,
);
impl WritingStorageError {
    pub(crate) fn into_inner(self) -> crate::data_interface::SqlImmediateFailure {
        *self.0
    }
    pub(crate) fn into_parts(
        self,
    ) -> (
        crate::data_interface::SqlImmediateFailure,
        Option<WritingSourceDiagnostic>,
    ) {
        (*self.0, self.1)
    }
    pub(crate) fn source_failure(
        failure: crate::ai::structured_resource_validation::Failure<
            crate::data_interface::SqlImmediateFailure,
        >,
        rejection: Option<WritingContextError>,
    ) -> WritingSubmissionError {
        let origin = diagnostics::source_diagnostic(failure.cause, rejection);
        WritingSubmissionError::Storage(Self(Box::new(failure.error), origin))
    }
    #[cfg(test)]
    pub(crate) fn as_inner(&self) -> &crate::data_interface::SqlImmediateFailure {
        &self.0
    }
}
impl From<crate::data_interface::SqlImmediateFailure> for WritingSubmissionError {
    fn from(cause: crate::data_interface::SqlImmediateFailure) -> Self {
        Self::Storage(WritingStorageError(Box::new(cause), None))
    }
}

#[derive(Debug, serde::Serialize, thiserror::Error)]
#[serde(transparent)]
#[error(transparent)]
pub struct WritingNativeError(Box<crate::runtime_plugins::native::NativeHostError>);
impl From<Box<crate::runtime_plugins::native::NativeHostError>> for WritingSubmissionError {
    fn from(cause: Box<crate::runtime_plugins::native::NativeHostError>) -> Self {
        Self::Native(WritingNativeError(cause))
    }
}
#[derive(Debug, thiserror::Error)]
pub enum WritingSubmissionError {
    #[error(transparent)]
    Native(WritingNativeError),
    #[error(transparent)]
    Storage(WritingStorageError),
    #[error("writing preparation database operation failed")]
    Sql(#[from] rusqlite::Error),
    #[error("conversation is not bound to the active manuscript")]
    Unbound,
    #[error("writing submission preparation unavailable")]
    Unavailable,
    #[error(transparent)]
    Instructions(#[from] super::instructions::InstructionReadError),
    #[error(transparent)]
    Context(#[from] WritingContextError),
    #[error(transparent)]
    Model(#[from] AgentModelPreparationError),
    #[error(transparent)]
    Input(#[from] SubmissionError),
}
impl WritingSubmissionError {
    pub(crate) fn into_source_tool_output(
        self,
        record: &SubmissionRecord,
        call: &research_agent_core::model::ToolCall,
    ) -> research_agent_core::runner::ToolOutput {
        let (error, source) = self.split_source();
        research_agent_core::runner::ToolOutput {
            content: serde_json::json!({"error":"writing_source_read_unavailable", "message":error.to_string(),
                "requestId":record.request_id,"turnId":record.turn_id,"callId":call.call_id,
                "writingSource":source}).to_string(),
            is_error: true,
        }
    }

    pub(crate) fn split_source(self) -> (Self, Option<WritingSourceDiagnostic>) {
        match self {
            Self::Native(error) => (
                Self::Unavailable,
                Some(WritingSourceDiagnostic::NativeFailure { error }),
            ),
            Self::Storage(error) => {
                let (cause, source) = error.into_parts();
                (Self::from(cause), source)
            }
            error => (error, None),
        }
    }
}
#[derive(Clone)]
pub struct WritingSubmissionService {
    store: AppStore,
    model_resolver: Option<AgentModelResolver>,
    worker_retention: super::repository::WorkerRetention,
    runtime_context: Option<crate::runtime_plugins::writing_projects::context::Binding>,
}
impl WritingSubmissionService {
    pub(crate) fn from_app_store(store: &AppStore) -> Self {
        Self {
            store: store.clone(),
            model_resolver: None,
            worker_retention: super::repository::WorkerRetention::default(),
            runtime_context: store.native_writing_context().cloned(),
        }
    }

    pub(crate) fn with_model_resolver(mut self, models: AgentModelResolver) -> Self {
        self.model_resolver = Some(models);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_runtime_context(
        mut self,
        binding: crate::runtime_plugins::writing_projects::context::Binding,
    ) -> Self {
        self.store = self.store.with_native_writing_context(binding.clone());
        self.runtime_context = Some(binding);
        self
    }
    #[cfg(any(
        target_os = "macos",
        all(debug_assertions, feature = "development-data")
    ))]
    pub(crate) fn worker_retention(&self) -> super::repository::WorkerRetention {
        self.worker_retention.clone()
    }
    /// Restore the original frozen input without consulting current editor/files
    /// or model defaults. This prepares native credentials, never reruns a turn.
    pub async fn restore(
        &self,
        models: &AgentModelResolver,
        receipt: WritingSubmissionReceipt,
    ) -> Result<PreparedWritingSubmission, WritingSubmissionError> {
        let record = self.restore_record(receipt).await?;
        let model = models
            .prepare_retained(
                AgentHost::Writing,
                Some(AgentModelSelection {
                    provider_id: record.input.model.provider_id.clone(),
                    model_id: record.input.model.model_id.clone(),
                    reasoning_effort: record.input.reasoning_effort.clone(),
                    service_tier: record.input.service_tier.clone(),
                }),
                self.worker_retention.clone(),
            )
            .await?;
        if model.identity() != &record.input.model
            || model.reasoning_effort() != record.input.reasoning_effort.as_deref()
            || model.service_tier() != record.input.service_tier.as_deref()
        {
            return Err(SubmissionError::Conflict.into());
        }
        let store = self.store.clone();
        self.worker_retention
            .spawn(move || {
                let db = store
                    .connection()
                    .map_err(|_| WritingSubmissionError::Unavailable)?;
                check_binding(&db, receipt.conversation_id, receipt.manuscript_id)
            })
            .await
            .map_err(|_| WritingSubmissionError::Unavailable)??;
        Ok(PreparedWritingSubmission { record, model })
    }

    /// Restore and validate the frozen receipt before any credential preparation.
    pub(crate) async fn restore_record(
        &self,
        receipt: WritingSubmissionReceipt,
    ) -> Result<SubmissionRecord, WritingSubmissionError> {
        let store = self.store.clone();
        self.worker_retention
            .spawn(move || {
                let db = store
                    .connection()
                    .map_err(|_| WritingSubmissionError::Unavailable)?;
                check_binding(&db, receipt.conversation_id, receipt.manuscript_id)?;
                let record = read(&db, receipt.conversation_id, receipt.request_id)?
                    .ok_or(SubmissionError::NotFound)?;
                validate_scope(&record, receipt.manuscript_id)?;
                Ok::<_, WritingSubmissionError>(record)
            })
            .await
            .map_err(|_| WritingSubmissionError::Unavailable)?
    }

    /// Checks binding before source/credential reads, then again in the insertion
    /// transaction. Dropping a waiter cannot start inference or mutate source files.
    pub async fn prepare(
        &self,
        models: &AgentModelResolver,
        request: WritingSubmissionRequest,
    ) -> Result<PreparedWritingSubmission, WritingSubmissionError> {
        self.prepare_with_compaction_fence(models, request, None)
            .await
    }

    async fn prepare_with_compaction_fence(
        &self,
        models: &AgentModelResolver,
        request: WritingSubmissionRequest,
        compact_revision: Option<u64>,
    ) -> Result<PreparedWritingSubmission, WritingSubmissionError> {
        let (record, model) = self
            .prepare_owned(models, request, compact_revision)
            .await?;
        // Native execution callers need a live handle. Preview/status replay
        // returns only the frozen record and does not resolve current credentials.
        let model = match model {
            Some(model) => model,
            None => self.model_for_record(models, &record).await?,
        };
        Ok(PreparedWritingSubmission { record, model })
    }
}
pub(crate) fn validate_scope(
    record: &SubmissionRecord,
    manuscript_id: Uuid,
) -> Result<(), WritingSubmissionError> {
    if record.input.context.is_empty() {
        return Err(SubmissionError::Corrupt.into());
    }
    let mut retrieval_trace = None;
    for source in &record.input.context {
        let anchor: serde_json::Value =
            serde_json::from_str(source.anchor.as_deref().ok_or(SubmissionError::Corrupt)?)
                .map_err(|_| SubmissionError::Corrupt)?;
        if anchor
            .get("supplementalContext")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            continue;
        }
        let manuscript_matches = anchor
            .get("manuscriptId")
            .and_then(serde_json::Value::as_str)
            == Some(manuscript_id.to_string().as_str());
        match source.kind {
            research_agent_core::submission::ContextSourceKind::Manuscript => {
                if !manuscript_matches {
                    return Err(WritingSubmissionError::Unbound);
                }
            }
            research_agent_core::submission::ContextSourceKind::Retrieval => {
                let trace = anchor
                    .get("traceId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(SubmissionError::Corrupt)?;
                if !manuscript_matches
                    || source.source_id != format!("retrieval:{trace}")
                    || source.revision != format!("trace:{trace}")
                    || retrieval_trace.replace(trace.to_owned()).is_some()
                {
                    return Err(WritingSubmissionError::Unbound);
                }
            }
            _ => return Err(WritingSubmissionError::Unbound),
        }
    }
    Ok(())
}

pub(crate) fn check_binding(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
) -> Result<(), WritingSubmissionError> {
    let allowed: bool = db.query_row("select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id join manuscripts m on m.id=b.resource_id where b.conversation_id=?1 and b.resource_kind='manuscript' and b.resource_id=?2 and c.archived=0 and m.status='active')",
        params![conversation.to_string(), manuscript.to_string()], |row| row.get(0)).map_err(WritingSubmissionError::Sql)?;
    if allowed {
        Ok(())
    } else {
        Err(WritingSubmissionError::Unbound)
    }
}
#[cfg(test)]
#[path = "writing_submission/tests.rs"]
mod tests;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use tests::{
    Fixture as WritingTestFixture, fixture as writing_test_fixture, request as writing_test_request,
};

#[path = "writing_submission/compaction_tools.rs"]
mod compaction_tools;
#[path = "writing_submission/execution.rs"]
mod execution;
#[path = "writing_submission/input_queue.rs"]
mod input_queue;
#[path = "writing_submission/queue_preview.rs"]
mod queue_preview;
#[path = "writing_submission/queued_compaction.rs"]
mod queued_compaction;
#[path = "writing_submission/skill_tools.rs"]
mod skill_tools;
#[path = "writing_submission/tools.rs"]
mod tools;
pub(crate) use compaction_tools::compaction_tool_catalog;
#[cfg(unix)]
#[path = "writing_submission/workspace.rs"]
pub mod workspace;
pub(crate) use queue_preview::WritingQueuePreviewError;
#[path = "writing_submission/steering.rs"]
mod steering;
pub use execution::{WritingExecutionConfirmation, WritingExecutionProfile, WritingStartError};
