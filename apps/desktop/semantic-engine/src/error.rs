use research_semantic_engine_api::EngineErrorResponse;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum EngineError {
    #[error("invalid semantic engine input: {0}")]
    InvalidInput(&'static str),
    #[error("semantic engine request is incompatible")]
    IncompatibleRequest,
    #[error("semantic engine Arrow schema is incompatible")]
    IncompatibleSchema,
    #[error("semantic engine Arrow data is invalid")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("LanceDB operation failed")]
    LanceDb(#[from] lancedb::Error),
    #[error("semantic engine JSON payload is invalid")]
    Json(#[from] serde_json::Error),
    #[error("semantic engine runtime could not start")]
    Runtime(#[from] std::io::Error),
}

impl EngineError {
    pub(crate) fn response(&self) -> EngineErrorResponse {
        let code = match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::IncompatibleRequest => "incompatible_request",
            Self::IncompatibleSchema | Self::Arrow(_) => "incompatible_schema",
            Self::LanceDb(_) => "lancedb_error",
            Self::Json(_) => "invalid_json",
            Self::Runtime(_) => "runtime_error",
        };
        EngineErrorResponse {
            code: code.to_owned(),
            message: self.to_string(),
        }
    }
}
