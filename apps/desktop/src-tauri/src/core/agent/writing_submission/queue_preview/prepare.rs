//! Exact prepare proof precedes credentials and every mutable source read.
use super::*;
use crate::data_interface::{
    QueuePreviewPrepareIntent, QueuePreviewPrepareSource, QueuePreviewPrepareStatus,
    SqlImmediateDiagnostic, SqlImmediateFailure,
};

impl WritingSubmissionService {
    pub(crate) async fn prepare_queue_preview(
        &self,
        models: &AgentModelResolver,
        intent: QueuePreviewPrepareIntent,
        request: WritingSubmissionRequest,
    ) -> Result<PreparedWritingQueuePreview, WritingQueuePreviewError> {
        let operation = intent.request_id;
        let database = self.store.shared_agent_connection();
        let replay_intent = intent.clone();
        if let Some(state) = self
            .worker_retention
            .spawn(move || database.replay_queue_preview_prepare(&replay_intent))
            .await
            .map_err(|_| worker_failure(operation, false))??
        {
            return Ok(state);
        }
        let prepared = models
            .prepare_retained(
                AgentHost::Writing,
                request.model.clone(),
                self.worker_retention.clone(),
            )
            .await
            .map_err(|cause| model_selection_failure(operation, cause))?;
        let source = QueuePreviewPrepareSource {
            store: self.store.clone(),
            request,
            model: FrozenWritingModel {
                identity: prepared.identity().clone(),
                reasoning_effort: prepared.reasoning_effort().map(str::to_owned),
                service_tier: prepared.service_tier().map(str::to_owned),
            },
        };
        let database = self.store.shared_agent_connection();
        if let Some(binding) = &self.runtime_context {
            return binding
                .preview(database, intent, source, self.worker_retention.clone())
                .await
                .map_err(Into::into);
        }
        // The lease belongs to the actual transaction worker after its waiter
        // is dropped; no async parent can release admission before commit ends.
        Ok(self
            .worker_retention
            .spawn(move || database.prepare_queue_preview(&intent, source))
            .await
            .map_err(|_| worker_failure(operation, true))??)
    }

    pub(crate) async fn read_queue_preview_prepare_status(
        &self,
        intent: QueuePreviewPrepareIntent,
    ) -> Result<QueuePreviewPrepareStatus, SqlImmediateFailure> {
        let operation = intent.request_id;
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.read_queue_preview_prepare_status(&intent))
            .await
            .map_err(|_| worker_failure(operation, false))?
    }
}

fn model_selection_failure(
    operation_id: Uuid,
    cause: AgentModelPreparationError,
) -> SqlImmediateFailure {
    // This invocation has not spawned its typed write. Only this fixed phase
    // and these safe classifications establish that pre-write boundary; other
    // unavailable failures cannot be treated as proof of non-commit.
    let safe_cause = match cause {
        AgentModelPreparationError::Settings => "model_settings_unavailable",
        AgentModelPreparationError::Disabled => "model_provider_disabled",
        AgentModelPreparationError::Credentials => "model_credentials_unavailable",
        AgentModelPreparationError::ChatGptTransportUnavailable => "chatgpt_transport_unavailable",
        AgentModelPreparationError::UnsupportedProtocol => "model_protocol_unsupported",
        AgentModelPreparationError::Worker => "model_preparation_worker_unavailable",
        AgentModelPreparationError::Transport(_) => "model_transport_unavailable",
    };
    SqlImmediateFailure::Unavailable(Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase: "queue-prepare-model-selection",
        sqlite_code: None,
        sqlite_extended_code: None,
        safe_cause,
    }))
}

fn worker_failure(operation_id: Uuid, writing: bool) -> SqlImmediateFailure {
    let detail = Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase: if writing {
            "queue-preview-prepare"
        } else {
            "queue-preview-prepare-status"
        },
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

#[cfg(test)]
#[path = "prepare/tests.rs"]
mod tests;
