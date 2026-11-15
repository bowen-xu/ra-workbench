//! Queued compact uses the same source materializer and the existing data owner.
use super::*;
use crate::{
    agent::{
        repository::input_queue::queued_compaction::{
            PrepareIntent, PrepareSource, QueuedCompactionRead, QueuedCompactionState, ReadIntent,
            SettleIntent, SettleState,
        },
        writing_queued_compaction::Error,
    },
    data_interface::{SqlImmediateDiagnostic, SqlImmediateFailure},
};

impl WritingSubmissionService {
    pub(crate) async fn prepare_queued_compaction(
        &self,
        models: &AgentModelResolver,
        intent: PrepareIntent,
        mut request: WritingSubmissionRequest,
    ) -> Result<QueuedCompactionState, Error> {
        if request.text.trim() != "/compact"
            || request.conversation_id != intent.conversation_id
            || request.request_id != intent.request_id
            || request.context.manuscript_id != intent.manuscript_id
        {
            return Err(Error::Invalid);
        }
        // Exact original proof wins before current model configuration, source
        // files or references can reinterpret a lost preparation response.
        let database = self.store.shared_agent_connection();
        let original = intent.clone();
        let operation = intent.request_id;
        let conversation = intent.conversation_id;
        let request_id = Some(intent.request_id);
        if let Some(saved) = self
            .worker_retention
            .spawn(move || database.replay_queued_compaction_prepare(&original))
            .await
            .map_err(|_| {
                worker_failure(
                    conversation,
                    request_id,
                    operation,
                    false,
                    "queued-compact-replay",
                )
            })?
            .map_err(|error| Error::data(error, conversation, request_id, operation))?
        {
            return Ok(saved);
        }
        let prepared = models
            .prepare_retained(
                AgentHost::Writing,
                request.model.take(),
                self.worker_retention.clone(),
            )
            .await
            .map_err(WritingSubmissionError::from)
            .map_err(Error::preparation)?;
        let model = FrozenWritingModel {
            identity: prepared.identity().clone(),
            reasoning_effort: prepared.reasoning_effort().map(str::to_owned),
            service_tier: prepared.service_tier().map(str::to_owned),
        };
        let database = self.store.shared_agent_connection();
        let source = PrepareSource {
            store: self.store.clone(),
            request,
            model,
        };
        if let Some(binding) = &self.runtime_context {
            return binding
                .compact(
                    database,
                    intent.clone(),
                    source,
                    self.worker_retention.clone(),
                )
                .await
                .map_err(|error| Error::source(error, &intent));
        }
        self.worker_retention
            .spawn(move || database.prepare_queued_compaction(&intent, source))
            .await
            .map_err(|_| {
                worker_failure(
                    conversation,
                    request_id,
                    operation,
                    true,
                    "queued-compact-prepare",
                )
            })?
            .map_err(|error| Error::data(error, conversation, request_id, operation))
    }

    pub(crate) async fn read_queued_compaction(
        &self,
        intent: ReadIntent,
    ) -> Result<Option<QueuedCompactionRead>, Error> {
        let database = self.store.shared_agent_connection();
        let operation = intent.request_id.unwrap_or(intent.input_id);
        let conversation = intent.conversation_id;
        let request_id = intent.request_id;
        self.worker_retention
            .spawn(move || database.read_queued_compaction(&intent))
            .await
            .map_err(|_| {
                worker_failure(
                    conversation,
                    request_id,
                    operation,
                    false,
                    "queued-compact-read",
                )
            })?
            .map_err(|error| Error::data(error, conversation, request_id, operation))
    }

    pub(crate) async fn settle_queued_compaction(
        &self,
        intent: SettleIntent,
    ) -> Result<SettleState, Error> {
        let operation = intent.operation_id;
        let conversation = intent.conversation_id;
        let request_id = Some(intent.request_id);
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.settle_queued_compaction(&intent))
            .await
            .map_err(|_| {
                worker_failure(
                    conversation,
                    request_id,
                    operation,
                    true,
                    "queued-compact-settle",
                )
            })?
            .map_err(|error| Error::data(error, conversation, request_id, operation))
    }
}

fn worker_failure(
    conversation: Uuid,
    request: Option<Uuid>,
    operation_id: Uuid,
    writing: bool,
    phase: &'static str,
) -> Error {
    let diagnostic = Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase,
        sqlite_code: None,
        sqlite_extended_code: None,
        safe_cause: "queued_compact_worker_result_unavailable",
    });
    let failure = if writing {
        SqlImmediateFailure::Unknown(diagnostic)
    } else {
        SqlImmediateFailure::Unavailable(diagnostic)
    };
    Error::data(failure, conversation, request, operation_id)
}
