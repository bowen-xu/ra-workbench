//! Replay immutable proof before crossing any credential or source boundary.
use super::*;
use crate::data_interface::{
    SqlImmediateDiagnostic, SqlImmediateFailure, WritingPrepareIntent, WritingPrepareSource,
    WritingPrepareStatus,
};

impl WritingSubmissionRequest {
    pub(crate) fn wire_request(&self) -> super::super::writing_commands::PrepareRequest {
        use super::super::{config::ConfigOverride, model::ModelSelection};
        let patch = |value: &InstructionConfigPatch| ConfigOverride {
            max_bytes: value.max_bytes,
            fallback_filenames: value.fallback_filenames.clone(),
        };
        super::super::writing_commands::PrepareRequest {
            conversation_id: self.conversation_id,
            request_id: self.request_id,
            text: self.text.clone(),
            user_input: self.user_input.clone(),
            context: self.context.clone(),
            resources: self.resources.clone(),
            retrieval: self.retrieval.clone(),
            model: self.model.as_ref().map(|value| ModelSelection {
                provider_id: value.provider_id.clone(),
                model_id: value.model_id.clone(),
                reasoning_effort: value.reasoning_effort.clone(),
                service_tier: value.service_tier.clone(),
            }),
            session_config: self.session_config.as_ref().map(patch),
            turn_config: self.turn_config.as_ref().map(patch),
        }
    }
}
impl WritingSubmissionService {
    pub(crate) async fn prepare_preview(
        &self,
        models: &AgentModelResolver,
        request: WritingSubmissionRequest,
        compact_revision: Option<u64>,
    ) -> Result<SubmissionRecord, WritingSubmissionError> {
        self.prepare_owned(models, request, compact_revision)
            .await
            .map(|(record, _)| record)
    }
    pub(crate) async fn read_prepare_status(
        &self,
        intent: WritingPrepareIntent,
    ) -> Result<WritingPrepareStatus, SqlImmediateFailure> {
        let operation = intent.operation();
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.writing_prepare_status(&intent))
            .await
            .map_err(|_| worker_failure(operation, false))?
    }
    pub(super) async fn prepare_owned(
        &self,
        models: &AgentModelResolver,
        request: WritingSubmissionRequest,
        compact_revision: Option<u64>,
    ) -> Result<(SubmissionRecord, Option<PreparedAgentModel>), WritingSubmissionError> {
        if request
            .retrieval
            .as_ref()
            .is_some_and(|value| value.validate().is_err())
        {
            return Err(SubmissionError::InvalidInput.into());
        }
        let intent = WritingPrepareIntent {
            request: request.wire_request(),
            compact_revision,
        };
        let operation = intent.operation();
        let replay = intent.clone();
        let database = self.store.shared_agent_connection();
        if let Some(record) = self
            .worker_retention
            .spawn(move || database.replay_writing_prepare(&replay))
            .await
            .map_err(|_| WritingSubmissionError::from(worker_failure(operation, false)))?
            .map_err(WritingSubmissionError::from)?
        {
            return Ok((record, None));
        }
        let model = models
            .prepare_retained(
                AgentHost::Writing,
                request.model.clone(),
                self.worker_retention.clone(),
            )
            .await?;
        let source = WritingPrepareSource {
            store: self.store.clone(),
            request,
            model: FrozenWritingModel {
                identity: model.identity().clone(),
                reasoning_effort: model.reasoning_effort().map(str::to_owned),
                service_tier: model.service_tier().map(str::to_owned),
            },
        };
        let database = self.store.shared_agent_connection();
        let record = match &self.runtime_context {
            Some(binding) => {
                binding
                    .prepare(database, intent, source, self.worker_retention.clone())
                    .await?
            }
            None => self
                .worker_retention
                .spawn(move || database.prepare_writing(&intent, source))
                .await
                .map_err(|_| WritingSubmissionError::from(worker_failure(operation, true)))?
                .map_err(WritingSubmissionError::from)?,
        };
        let matches = model.identity() == &record.input.model
            && model.reasoning_effort() == record.input.reasoning_effort.as_deref()
            && model.service_tier() == record.input.service_tier.as_deref();
        Ok((record, matches.then_some(model)))
    }
    pub(super) async fn model_for_record(
        &self,
        models: &AgentModelResolver,
        record: &SubmissionRecord,
    ) -> Result<PreparedAgentModel, WritingSubmissionError> {
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
        Ok(model)
    }
}
pub(crate) fn worker_failure(operation_id: Uuid, writing: bool) -> SqlImmediateFailure {
    let detail = Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase: "writing-prepare-worker",
        sqlite_code: None,
        sqlite_extended_code: None,
        safe_cause: "worker_join_failed",
    });
    if writing {
        SqlImmediateFailure::Unknown(detail)
    } else {
        SqlImmediateFailure::Unavailable(detail)
    }
}
