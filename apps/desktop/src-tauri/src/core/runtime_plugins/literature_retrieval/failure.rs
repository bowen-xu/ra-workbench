//! Preserve original retrieval categories alongside safe finite source diagnostics.
use crate::{
    ai::{
        EmbeddingCallError,
        knowledge_contracts::{ConceptRetrievalError, SemanticIndexError},
        structured_resource_validation::{DecisionOrigin, Failure, FailureCause},
    },
    data_interface::plugin_resources::PluginDataError,
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{literature_hbs::HbsOperationError, native::NativeHostError},
};
use serde_json::{Value, json};

#[derive(Debug)]
pub(crate) enum Error {
    Boundary(Box<BoundaryFailure>),
    Original(ConceptRetrievalError),
    Embedding(EmbeddingCallError),
    Data(Box<PluginDataError>),
    Rejected {
        message: String,
        origin: DecisionOrigin,
    },
    Native(Box<NativeHostError>),
    Unsettled,
}
impl From<ConceptRetrievalError> for Error {
    fn from(error: ConceptRetrievalError) -> Self {
        Self::Original(error)
    }
}
impl From<EmbeddingCallError> for Error {
    fn from(error: EmbeddingCallError) -> Self {
        Self::Embedding(error)
    }
}
impl From<Box<PluginDataError>> for Error {
    fn from(error: Box<PluginDataError>) -> Self {
        Self::Data(error)
    }
}
impl From<Box<NativeHostError>> for Error {
    fn from(error: Box<NativeHostError>) -> Self {
        Self::Native(error)
    }
}
impl Error {
    pub fn into_wire(
        self,
        client: Option<&str>,
        trace: Option<&str>,
        phase: &'static str,
        outcome: HbsOutcome,
    ) -> Value {
        let (code, message, cause) = match self {
            Self::Original(error) => {
                let cause = match &error {
                    ConceptRetrievalError::SemanticIndex(error) => Some(match error {
                        SemanticIndexError::InvalidInput(_) => "invalid_input",
                        SemanticIndexError::EngineLoad(_) => "engine_load_failed",
                        SemanticIndexError::EngineAbiMismatch => "engine_abi_mismatch",
                        SemanticIndexError::EngineProtocol(_) => "engine_protocol_invalid",
                        SemanticIndexError::EngineOperation { code } => code.as_str(),
                        SemanticIndexError::Database(_) => "database_failed",
                        SemanticIndexError::Storage(_) => "storage_unavailable",
                        SemanticIndexError::WorkerUnavailable => "worker_unavailable",
                        SemanticIndexError::Canceled => "canceled",
                        SemanticIndexError::Provider(code) => code,
                    }),
                    _ => None,
                };
                let mut value = original_message(&error);
                if let Some(code) = cause {
                    // Engine operation codes are bounded technical tokens. Opaque
                    // loader/protocol text and payloads never become diagnostics.
                    if !code.is_empty()
                        && code.len() <= 100
                        && code
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        value["semanticError"] = json!({"code":code});
                    }
                }
                return attach(value, client, trace);
            }
            Self::Embedding(error) => {
                let mut value = original(ConceptRetrievalError::Provider);
                value["providerError"] = match error {
                    EmbeddingCallError::Configuration => json!({"code":"embedding_configuration"}),
                    EmbeddingCallError::Provider(error) => {
                        let mut cause = json!({"code":error.kind.code()});
                        if let Some(delay) = error.retry_after_ms {
                            cause["retryAfterMs"] = json!(delay);
                        }
                        cause
                    }
                };
                return attach(value, client, trace);
            }
            Self::Boundary(error) => {
                let (message, diagnostic) = HbsOperationError::Boundary(*error).into_wire();
                return attach(
                    json!({"code":"storage_error","message":message,"nativeFailure":diagnostic}),
                    client,
                    trace,
                );
            }
            Self::Data(error) => (
                "storage_error",
                "概念网络数据库操作失败".to_owned(),
                Some(FailureCause::SourceData(error)),
            ),
            Self::Rejected { message, origin } => (
                "invalid_input",
                message,
                Some(FailureCause::NativeRejection(Box::new(origin))),
            ),
            Self::Native(mut error) => {
                if error.operation_id.is_none() {
                    error.operation_id = trace.map(str::to_owned);
                }
                let code = error.code.clone();
                let (message, diagnostic) =
                    HbsOperationError::native(error, phase, outcome).into_wire();
                return attach(
                    json!({"code":code,"message":message,"nativeFailure":diagnostic}),
                    client,
                    trace,
                );
            }
            Self::Unsettled => (
                "worker_unavailable",
                "概念网络检索工作未确认结算，请核对原请求与预览。".into(),
                None,
            ),
        };
        let mut value = json!({"code":code,"message":message});
        if let Some(cause) = cause {
            let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                failure: Failure {
                    error: message,
                    cause,
                },
                phase,
                outcome,
                rollback_error: None,
            })
            .into_wire();
            value["message"] = json!(message);
            value["nativeFailure"] = json!(diagnostic);
        } else {
            value["outcome"] = json!("unknown");
        }
        attach(value, client, trace)
    }
}
fn attach(mut value: Value, client: Option<&str>, trace: Option<&str>) -> Value {
    value["retrievalOperation"] = json!("preview");
    if let Some(client) = client {
        value["clientRequestId"] = json!(client);
    }
    if let Some(trace) = trace {
        value["traceId"] = json!(trace);
    }
    value
}
pub(super) fn original(error: ConceptRetrievalError) -> Value {
    original_message(&error)
}
fn original_message(error: &ConceptRetrievalError) -> Value {
    let code = match error {
        ConceptRetrievalError::InvalidInput(_) => "invalid_input",
        ConceptRetrievalError::NotFound => "not_found",
        ConceptRetrievalError::Expired => "expired",
        ConceptRetrievalError::QueryMismatch => "query_mismatch",
        ConceptRetrievalError::AlreadyConsumed => "already_consumed",
        ConceptRetrievalError::Provider => "provider_unavailable",
        ConceptRetrievalError::Canceled => "canceled",
        ConceptRetrievalError::SemanticIndex(_) => "semantic_index_unavailable",
        ConceptRetrievalError::Database(_) | ConceptRetrievalError::Storage(_) => "storage_error",
        ConceptRetrievalError::WorkerUnavailable => "worker_unavailable",
    };
    json!({"code":code,"message":error.to_string()})
}
