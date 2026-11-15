//! Compatibility adapter from the legacy paper input shape to the shared Agent
//! Responses transport. The transport, authentication lease, SSE decoder and
//! cancellation behavior remain owned by the shared Agent implementation.
#[path = "paper_reading_shared_transport/prepared.rs"]
mod prepared;
#[path = "paper_reading_shared_transport/submissions.rs"]
mod submissions;
#[cfg(test)]
#[path = "paper_reading_shared_transport/tool_loop_tests.rs"]
mod tool_loop_tests;
#[path = "paper_reading_shared_transport/workflow.rs"]
mod workflow;
use prepared::{
    call_prepared_model, legacy_submission_text, map_ai_error, resolve_submission_completion,
};
pub(super) use prepared::{call_prepared_text, map_model_error, provider_messages};
pub(super) use submissions::{call_prepared_text_submission, call_prepared_vision_submission};

use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD};
use research_agent_core::model::{
    ContentPart, ModelCapabilities, ModelError, ModelMessage, ModelRequest, ModelTransport,
    ToolDefinition,
};
use research_agent_core::runner::{
    RunError, RunFuture, RunIdSource, RunLimits, ToolExecutor, TurnEventSink, TurnRunner,
};
use research_agent_core::submission::{SubmissionInput, SubmissionStore};
use research_agent_core::{conversation::ConversationId, protocol::EventPayload};
use serde_json::json;
use uuid::Uuid;

use crate::agent::model::PreparedAgentModel;
use crate::agent::model::responses_continuation::ResponsesContinuation;
use crate::agent::model::{ChatRequestPolicy, OutputTokenParameter};
use crate::agent::{
    repository::submissions::SqliteSubmissionStore,
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use crate::storage::AppStore;

use super::{
    error::AiError,
    paper_reading::{ProviderInput, ProviderMessageRole, VisionGenerationResponse},
    provider::CancellationSignal,
};

#[cfg(test)]
pub(in crate::ai) async fn prepare_workflow_for_test(
    store: &AppStore,
    conversation: Uuid,
    request: Uuid,
    frozen: Option<&SubmissionInput>,
) -> Result<Option<Box<dyn super::generation_hooks::PreparedPaperWorkflow>>, AiError> {
    workflow::prepare(store, conversation, request, frozen).await
}

const MAX_MODEL_INPUT_BYTES: usize = 32 * 1024 * 1024;

/// Optional host-owned projection for a normalized text stream. The shared
/// transport does not know about paper sessions, Tauri events, or UI state;
/// those remain an adapter concern at the call site.
pub(super) trait VisionStreamSink: Send + Sync {
    fn default_assistant_message_id(&self) -> String;
    fn emit_assistant(
        &self,
        status: &str,
        message_id: &str,
        delta: Option<String>,
        accumulated: &str,
    );
    fn emit(
        &self,
        status: &str,
        message_id: Option<String>,
        delta: Option<String>,
        accumulated: &str,
    );
    fn has_assistant_message(&self) -> bool;
}

struct LocalIds;
impl RunIdSource for LocalIds {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

struct ProjectionSink {
    stream_sink: Option<Arc<dyn VisionStreamSink>>,
    durable: Option<DurableProjection>,
    accumulated: Mutex<String>,
}
#[derive(Clone)]
struct DurableProjection {
    service: Arc<AgentSessionService>,
    conversation_id: ConversationId,
    request_id: Uuid,
}
struct VisionRunOptions {
    stream_sink: Option<Arc<dyn VisionStreamSink>>,
    cancellation: Option<CancellationSignal>,
    durable: Option<DurableProjection>,
    turn_id: Option<Uuid>,
    agent_cancellation: Option<crate::agent::cancellation::AgentCancellation>,
    tool_executor: Option<Arc<dyn ToolExecutor>>,
    tool_definitions: Vec<ToolDefinition>,
}
impl TurnEventSink for ProjectionSink {
    fn emit(&self, payload: EventPayload) -> RunFuture<'_, ()> {
        if let (Some(sink), EventPayload::TextDelta { text, .. }) = (&self.stream_sink, &payload)
            && let Ok(mut accumulated) = self.accumulated.lock()
        {
            accumulated.push_str(text);
            let id = sink.default_assistant_message_id();
            sink.emit_assistant("delta", &id, Some(text.clone()), &accumulated);
        }
        let durable = self.durable.clone();
        Box::pin(async move {
            if let Some(durable) = durable {
                durable
                    .service
                    .commit_payload(durable.conversation_id, durable.request_id, payload)
                    .await
                    .map_err(|_| RunError::EventSink)?;
            }
            Ok(())
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn consume_vision_stream(
    transport: Arc<dyn ModelTransport>,
    request: ModelRequest,
    stream_sink: Option<Arc<dyn VisionStreamSink>>,
    cancellation: Option<crate::agent::cancellation::AgentCancellation>,
    transport_name: &'static str,
    durable: Option<DurableProjection>,
    turn_id: Option<Uuid>,
    tools: Arc<dyn ToolExecutor>,
) -> Result<VisionGenerationResponse, AiError> {
    let sink = ProjectionSink {
        stream_sink: stream_sink.clone(),
        durable,
        accumulated: Mutex::new(String::new()),
    };
    let ids = LocalIds;
    let has_tools = !request.tools.is_empty();
    let limits = RunLimits {
        // Let the shared Codex-derived retry policy handle transient provider
        // failures. A tool call still consumes the same bounded turn budget;
        // this is not a second provider loop in the paper adapter.
        model_calls: Some(if has_tools { 16 } else { 3 }),
        tool_calls: Some(if has_tools { 64 } else { 1 }),
        output_bytes: Some(8 * 1024 * 1024),
        tool_output_bytes: if has_tools { 2 * 1024 * 1024 } else { 1 },
        response: research_agent_core::model::StreamLimits {
            text_bytes: 8 * 1024 * 1024,
            argument_bytes: if has_tools { 1024 * 1024 } else { 1 },
            tool_calls: if has_tools { 32 } else { 1 },
        },
    };
    let cancellation = cancellation.unwrap_or_default();
    let request_id = request.request_id;
    let result = crate::agent::cancellation::run_cancellable_turn(
        TurnRunner {
            transport: transport.as_ref(),
            tools: tools.as_ref(),
            sink: &sink,
            ids: &ids,
            limits,
        },
        turn_id.unwrap_or(request_id),
        request,
        cancellation,
    )
    .await
    .map_err(map_run_error)?;
    let input_tokens = result
        .responses
        .iter()
        .map(|response| response.usage.input_tokens)
        .try_fold(0_u64, |sum, value| sum.checked_add(value?))
        .and_then(|value| i64::try_from(value).ok());
    let output_tokens = result
        .responses
        .iter()
        .map(|response| response.usage.output_tokens)
        .try_fold(0_u64, |sum, value| sum.checked_add(value?))
        .and_then(|value| i64::try_from(value).ok());
    let response = result
        .responses
        .into_iter()
        .last()
        .ok_or(AiError::Provider("invalid_response"))?;
    let content = response.text;
    if !has_tools && content.trim().is_empty() {
        return Err(AiError::Provider("invalid_response"));
    }
    if let Some(sink) = &stream_sink {
        if sink.has_assistant_message() {
            sink.emit("completed", None, None, &content);
        } else {
            let id = sink.default_assistant_message_id();
            sink.emit_assistant("completed", &id, Some(content.clone()), &content);
        }
    }
    Ok(VisionGenerationResponse {
        content: content.clone(),
        input_tokens,
        output_tokens,
        context_window_tokens: None,
        context_window_source: None,
        raw_response: json!({"transport":transport_name,"requestId":request_id,"content":content,"inputTokens":input_tokens,"outputTokens":output_tokens}),
    })
}

fn map_run_error(error: RunError) -> AiError {
    match error {
        RunError::ToolOutcomeUnknown => AiError::Provider("tool_outcome_unknown"),
        RunError::HostFinalizationFailed => AiError::Provider("host_finalization_failed"),
        RunError::ToolOutcomeUnknownDiagnostic(detail) => AiError::ToolOutcomeDiagnostic(detail),
        RunError::EventSinkDiagnostic(detail) => AiError::ToolOutcomeDiagnostic(detail),
        RunError::ToolNotFound => AiError::Provider("tool_not_found"),
        RunError::Model(error) => map_model_error(error),
        RunError::EventSink
        | RunError::ToolRejected
        | RunError::ToolRejectionDiagnostic(_)
        | RunError::ToolArguments(_)
        | RunError::ToolUnavailable
        | RunError::Budget => AiError::Provider("invalid_response"),
    }
}

/// Vision adapter for the shared model resolver. This is the migration seam
/// for structured paper workflows: provider credentials and transport policy
/// are frozen by `PreparedAgentModel`, while this compatibility layer only
/// translates the legacy multimodal input shape.
pub(super) async fn call_prepared_vision(
    prepared: PreparedAgentModel,
    input: ProviderInput,
    json_response: bool,
    stream_sink: Option<Arc<dyn VisionStreamSink>>,
    cancellation: Option<CancellationSignal>,
) -> Result<VisionGenerationResponse, AiError> {
    call_prepared_model(
        prepared,
        input,
        json_response,
        8_000,
        true,
        VisionRunOptions {
            stream_sink,
            cancellation,
            durable: None,
            turn_id: None,
            agent_cancellation: None,
            tool_executor: None,
            tool_definitions: Vec::new(),
        },
    )
    .await
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn detailed_unknown_survives_legacy_command_error_conversion() {
        let request_id = Uuid::new_v4();
        let detail = research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(json!({
            "code":"TEXT_PATCH_JOURNAL_UNCONFIRMED","requestId":request_id,
            "phase":"call-terminal","safeCause":"journal_sql_failed",
            "sqliteCode":"ConstraintViolation","sqliteExtendedCode":1811,
        }))
        .unwrap();
        let error: crate::ai::error::AiCommandError =
            map_run_error(RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail))).into();
        assert_eq!(error.code(), "tool_outcome_unknown");
        for expected in [
            request_id.to_string(),
            "TEXT_PATCH_JOURNAL_UNCONFIRMED".into(),
            "call-terminal".into(),
            "journal_sql_failed".into(),
            "1811".into(),
            "避免重复执行".into(),
        ] {
            assert!(error.message().contains(&expected), "{expected}");
        }
    }

    #[test]
    fn detailed_receipt_failure_survives_legacy_command_error_conversion() {
        let request_id = Uuid::new_v4();
        let detail = research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(json!({
            "code":"TOOL_RECEIPT_UNCONFIRMED","requestId":request_id,
            "phase":"result-ack","safeCause":"owner_ack_failed",
        }))
        .unwrap();
        let error: crate::ai::error::AiCommandError =
            map_run_error(RunError::EventSinkDiagnostic(Box::new(detail))).into();
        assert_eq!(error.code(), "tool_outcome_unknown");
        assert!(error.message().contains(&request_id.to_string()));
        assert!(error.message().contains("owner_ack_failed"));
    }
}
