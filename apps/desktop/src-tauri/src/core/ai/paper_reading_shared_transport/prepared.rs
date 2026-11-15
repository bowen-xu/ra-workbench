//! Prepared transport calls, legacy input conversion and sanitized model errors.
use super::*;

pub(super) fn legacy_submission_text(input: &ProviderInput) -> String {
    let mut text = input.system_text.clone().unwrap_or_default();
    for message in &input.messages {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&message.text);
    }
    text.chars().take(1_048_576).collect()
}

pub(super) fn map_ai_error(error: &AiError) -> RunError {
    let model = match error {
        AiError::Model { cause, .. } => *cause,
        AiError::Canceled => ModelError::Canceled,
        AiError::InvalidInput(_)
        | AiError::NativeWorkflow {
            code: "invalid_input",
            ..
        } => ModelError::InvalidRequest,
        AiError::CredentialMissing
        | AiError::CredentialInvalid
        | AiError::CredentialPermissions
        | AiError::CredentialFileType
        | AiError::CredentialIo(_)
        | AiError::CredentialLocation(_) => ModelError::Authentication,
        AiError::Provider("invalid_response") => ModelError::InvalidStream,
        _ => ModelError::Unavailable,
    };
    RunError::Model(model)
}

pub(super) fn resolve_submission_completion(
    completion: TaskCompletion,
    result: Option<Result<VisionGenerationResponse, AiError>>,
) -> Result<VisionGenerationResponse, AiError> {
    match completion {
        TaskCompletion::Completed => result.unwrap_or(Err(AiError::TemporaryUnavailable)),
        TaskCompletion::Canceled => Err(AiError::Canceled),
        TaskCompletion::Failed | TaskCompletion::RecoveryRequired => match result {
            Some(Err(error)) => Err(error),
            Some(Ok(_)) | None => Err(AiError::TemporaryUnavailable),
        },
    }
}

pub(in crate::ai) async fn call_prepared_text(
    prepared: PreparedAgentModel,
    input: ProviderInput,
    json_response: bool,
    max_output_tokens: u32,
    cancellation: Option<CancellationSignal>,
) -> Result<VisionGenerationResponse, AiError> {
    call_prepared_model(
        prepared,
        input,
        json_response,
        max_output_tokens,
        false,
        VisionRunOptions {
            stream_sink: None,
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

pub(super) async fn call_prepared_model(
    prepared: PreparedAgentModel,
    input: ProviderInput,
    json_response: bool,
    max_output_tokens: u32,
    vision: bool,
    options: VisionRunOptions,
) -> Result<VisionGenerationResponse, AiError> {
    let VisionRunOptions {
        stream_sink,
        cancellation,
        durable,
        turn_id,
        agent_cancellation,
        tool_executor,
        tool_definitions,
    } = options;
    let model = prepared.identity().clone();
    let reasoning_effort = prepared.reasoning_effort().map(str::to_owned);
    let capabilities = ModelCapabilities {
        vision,
        tools: tool_executor.is_some(),
        parallel_tools: false,
        reasoning_efforts: reasoning_effort.clone().into_iter().collect(),
    };
    let messages = provider_messages(input, json_response)?;
    if messages.is_empty() {
        return Err(AiError::InvalidInput("视觉请求没有可发送的内容".into()));
    }
    let request_id = Uuid::new_v4();
    let request = ModelRequest {
        captured_history: None,
        request_id,
        model: model.clone(),
        messages,
        tools: tool_definitions,
        max_output_tokens,
        parallel_tool_calls: false,
        reasoning_effort,
    };
    if prepared.is_chatgpt() {
        let continuation = Arc::new(
            ResponsesContinuation::new(request_id, model, MAX_MODEL_INPUT_BYTES)
                .map_err(map_model_error)?,
        );
        let transport: Arc<dyn ModelTransport> = Arc::new(
            prepared
                .into_chatgpt_transport(capabilities, continuation)
                .map_err(|error| {
                    AiError::Provider(match error {
                        crate::ai::agent_configuration::AgentModelPreparationError::Credentials => {
                            "authentication"
                        }
                        _ => "invalid_request",
                    })
                })?,
        );
        consume_vision_stream(
            transport,
            request,
            stream_sink,
            agent_cancellation.or_else(|| {
                cancellation.map(crate::agent::cancellation::AgentCancellation::from_signal)
            }),
            "prepared-agent-responses",
            durable,
            turn_id,
            tool_executor
                .unwrap_or_else(|| Arc::new(crate::agent::composed_tools::RejectingToolExecutor)),
        )
        .await
    } else {
        let policy = if json_response {
            ChatRequestPolicy::JsonObject
        } else {
            ChatRequestPolicy::ProviderDefault
        };
        let transport: Arc<dyn ModelTransport> = Arc::new(
            prepared
                .into_chat_transport_with_policy(
                    capabilities,
                    OutputTokenParameter::MaxTokens,
                    policy,
                )
                .map_err(|_| AiError::Provider("invalid_request"))?,
        );
        consume_vision_stream(
            transport,
            request,
            stream_sink,
            agent_cancellation.or_else(|| {
                cancellation.map(crate::agent::cancellation::AgentCancellation::from_signal)
            }),
            "prepared-agent-chat",
            durable,
            turn_id,
            tool_executor
                .unwrap_or_else(|| Arc::new(crate::agent::composed_tools::RejectingToolExecutor)),
        )
        .await
    }
}

pub(in crate::ai) fn provider_messages(
    input: ProviderInput,
    json_response: bool,
) -> Result<Vec<ModelMessage>, AiError> {
    let mut messages = Vec::new();
    if let Some(mut system) = input.system_text {
        if json_response {
            system.push_str("\nReturn one valid JSON object and no surrounding prose.");
        }
        messages.push(ModelMessage::System(system));
    }
    for message in input.messages {
        let mut parts = vec![ContentPart::Text(message.text)];
        for image in message.images {
            if !matches!(message.role, ProviderMessageRole::User) {
                return Err(AiError::Provider("invalid_request"));
            }
            parts.push(ContentPart::Text(image.context_text));
            parts.push(decode_image(&image.data_url)?);
            parts.push(ContentPart::Text(format!("[{}]", image.context_item_id)));
        }
        messages.push(match message.role {
            ProviderMessageRole::User => ModelMessage::User {
                content: parts,
                source_item_id: None,
            },
            ProviderMessageRole::Assistant => ModelMessage::Assistant {
                phase: None,
                content: parts,
                tool_calls: vec![],
            },
        });
    }
    Ok(messages)
}

fn decode_image(data_url: &str) -> Result<ContentPart, AiError> {
    let (metadata, encoded) = data_url
        .split_once(',')
        .ok_or(AiError::Provider("invalid_request"))?;
    let media_type = metadata
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .filter(|value| value == &"image/png")
        .ok_or(AiError::Provider("invalid_request"))?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| AiError::Provider("invalid_request"))?;
    if bytes.is_empty() || bytes.len() > MAX_MODEL_INPUT_BYTES {
        return Err(AiError::Provider("invalid_request"));
    }
    Ok(ContentPart::Image {
        media_type: media_type.to_owned(),
        bytes: Arc::from(bytes),
    })
}

pub(in crate::ai) fn map_model_error(error: ModelError) -> AiError {
    let mapped = match error {
        ModelError::Persistence | ModelError::PersistenceDiagnostic(_) => {
            return AiError::Model {
                code: "storage_error",
                cause: error,
            };
        }
        ModelError::Authentication | ModelError::HttpAuthentication(_) => {
            AiError::Provider("authentication")
        }
        ModelError::RateLimited | ModelError::HttpRateLimited(_) => {
            AiError::Provider("rate_limited")
        }
        ModelError::ContextWindowExceeded => AiError::Provider("context_window_exceeded"),
        ModelError::UsageLimitReached => AiError::Provider("usage_limit_reached"),
        ModelError::Canceled => AiError::Canceled,
        ModelError::Truncated => AiError::Provider("truncated_output"),
        ModelError::Filtered => AiError::Provider("content_filtered"),
        ModelError::UnsupportedCapability
        | ModelError::InvalidRequest
        | ModelError::ProviderRejected => AiError::Provider("invalid_request"),
        ModelError::Unavailable
        | ModelError::TimedOut
        | ModelError::ConnectionFailed
        | ModelError::ResponseReadFailed
        | ModelError::ServerOverloaded
        | ModelError::InternalServerError
        | ModelError::UnexpectedStatus(500..=599) => AiError::TemporaryUnavailable,
        ModelError::UnexpectedStatus(_) => AiError::Provider("unexpected_status"),
        ModelError::UnsupportedResponseType
        | ModelError::InvalidStream
        | ModelError::IncompleteStream
        | ModelError::BudgetExceeded => AiError::Provider("invalid_response"),
    };
    if matches!(error, ModelError::Canceled) {
        return mapped;
    }
    AiError::Model {
        code: mapped.code(),
        cause: error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_preserves_the_model_cause_instead_of_unavailable() {
        for cause in [
            ModelError::HttpAuthentication(401),
            ModelError::HttpAuthentication(403),
            ModelError::HttpRateLimited(429),
            ModelError::ConnectionFailed,
        ] {
            let error = map_model_error(cause);
            assert!(matches!(map_ai_error(&error), RunError::Model(restored) if restored==cause));
            assert!(error.user_message().contains(&cause.to_string()));
        }
    }

    #[test]
    fn unsupported_output_is_a_response_failure_not_a_request_capability_failure() {
        let error = map_model_error(ModelError::UnsupportedResponseType);
        assert_eq!(error.code(), "invalid_response");
        assert!(error.user_message().contains("无法解析"));
        assert!(!error.user_message().contains("模型 ID"));
    }

    #[test]
    fn canceled_shared_task_wins_over_a_dropped_result_channel() {
        match resolve_submission_completion(TaskCompletion::Canceled, None) {
            Err(error) => assert_eq!(error.code(), "canceled"),
            Ok(_) => panic!("canceled shared task must not return a response"),
        }
    }
    use crate::agent::repository::SqliteConversationStore;
    use crate::ai::paper_reading::{ProviderImage, ProviderMessage};
    use research_agent_core::protocol::{
        AgentEvent, EventPayload, ItemKind, ModelIdentity, TurnOutcome,
    };
    use rusqlite::Connection;

    fn durable_service() -> (Arc<AgentSessionService>, ConversationId) {
        let mut connection = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut connection, 67);
        let connection = Arc::new(Mutex::new(connection));
        let store =
            Arc::new(SqliteConversationStore::new(connection, "Paper transport".into()).unwrap());
        let id = ConversationId::from_uuid(Uuid::new_v4());
        (Arc::new(AgentSessionService::new(store)), id)
    }

    #[test]
    fn converts_legacy_input_without_losing_image_source_identity() {
        let input = ProviderInput {
            audit_text: "ignored".into(),
            system_text: Some("system".into()),
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                text: "inspect".into(),
                images: vec![ProviderImage {
                    context_item_id: "P1-F2".into(),
                    context_text: "figure".into(),
                    data_url: "data:image/png;base64,iVBORw0KGgo=".into(),
                }],
            }],
        };
        let messages = provider_messages(input, true).unwrap();
        assert!(matches!(&messages[0], ModelMessage::System(text) if text.contains("valid JSON")));
        let ModelMessage::User { content: parts, .. } = &messages[1] else {
            panic!("user message")
        };
        assert!(
            matches!(&parts[2], ContentPart::Image { media_type, bytes } if media_type == "image/png" && !bytes.is_empty())
        );
        assert!(matches!(&parts[3], ContentPart::Text(text) if text == "[P1-F2]"));
    }

    #[test]
    fn rejects_non_png_or_malformed_data_urls() {
        assert!(decode_image("data:image/jpeg;base64,AA==").is_err());
        assert!(decode_image("data:image/png;base64,not-base64").is_err());
    }

    #[tokio::test]
    async fn projection_sink_persists_normalized_events_to_shared_history() {
        let (service, conversation_id) = durable_service();
        service.create(conversation_id).await.unwrap();
        let turn_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        service
            .commit_payloads(
                conversation_id,
                request_id,
                vec![
                    EventPayload::TurnStarted {
                        task_kind: research_agent_core::protocol::TaskKind::Regular,
                        turn_id,
                        context_snapshot_id: Uuid::new_v4(),
                        model: ModelIdentity {
                            provider_id: "test".into(),
                            model_id: "paper-test".into(),
                            revision: None,
                        },
                    },
                    EventPayload::ItemStarted {
                        turn_id,
                        item_id,
                        kind: ItemKind::AssistantMessage,
                    },
                ],
            )
            .await
            .unwrap();

        let sink = ProjectionSink {
            stream_sink: None,
            durable: Some(DurableProjection {
                service: service.clone(),
                conversation_id,
                request_id,
            }),
            accumulated: Mutex::new(String::new()),
        };
        sink.emit(EventPayload::TextDelta {
            turn_id,
            item_id,
            text: "shared event".into(),
        })
        .await
        .unwrap();
        sink.emit(EventPayload::ItemCompleted { turn_id, item_id })
            .await
            .unwrap();
        sink.emit(EventPayload::TurnFinished {
            turn_id,
            outcome: TurnOutcome::Completed,
        })
        .await
        .unwrap();

        let history = service.read_since(conversation_id, 0, 20).await.unwrap();
        assert!(history.items.iter().any(|event: &AgentEvent| {
            matches!(event.payload(), EventPayload::TextDelta { text, .. } if text == "shared event")
        }));
        let projection = service.open(conversation_id).await.unwrap();
        assert_eq!(
            projection.turn(turn_id).and_then(|turn| turn.outcome),
            Some(TurnOutcome::Completed)
        );
    }
}
