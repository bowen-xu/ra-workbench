//! Direct service tests use the same decoded transport fingerprint as IPC.
use super::*;
use crate::agent::{
    config::ConfigOverride, model::ModelSelection, writing_commands::PrepareRequest,
    writing_queue_preview::QueuePreviewRequest,
};
use crate::data_interface::QueuePreviewPrepareIntent;

pub(in crate::agent::writing_submission) fn queue_preview_intent(
    request: &WritingSubmissionRequest,
    input_id: Uuid,
    revision: u64,
) -> QueuePreviewPrepareIntent {
    let patch = |patch: &InstructionConfigPatch| ConfigOverride {
        max_bytes: patch.max_bytes,
        fallback_filenames: patch.fallback_filenames.clone(),
    };
    let wire = QueuePreviewRequest {
        input_id,
        expected_revision: revision,
        submission: PrepareRequest {
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            text: request.text.clone(),
            user_input: request.user_input.clone(),
            context: request.context.clone(),
            resources: request.resources.clone(),
            retrieval: request.retrieval.clone(),
            model: request.model.as_ref().map(|model| ModelSelection {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
                reasoning_effort: model.reasoning_effort.clone(),
                service_tier: model.service_tier.clone(),
            }),
            session_config: request.session_config.as_ref().map(patch),
            turn_config: request.turn_config.as_ref().map(patch),
        },
    };
    QueuePreviewPrepareIntent {
        conversation_id: request.conversation_id,
        manuscript_id: request.context.manuscript_id,
        input_id,
        request_id: request.request_id,
        expected_queue_revision: revision,
        request_json: serde_json::to_value(wire).unwrap(),
    }
}
