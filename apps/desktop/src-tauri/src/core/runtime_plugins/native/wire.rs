//! Typed adaptation of exactly two SDK profiles over the original pipe exchange.
use research_workbench_native_sdk::{
    literature_trace_protocol as literature, protocol as writing,
    writing_projects_protocol as projects,
};

#[derive(Clone, Debug)]
pub(super) enum WireRequest {
    Writing(writing::Request),
    Literature(literature::Request),
    Projects(projects::Request),
}
#[derive(Debug)]
pub(super) enum WireResponse {
    Writing(writing::ResponseBody),
    Literature(literature::ResponseBody),
    Catalog(research_workbench_native_sdk::catalog::CatalogOutput),
    ProjectBody(projects::ResponseBody),
    Projects(research_workbench_native_sdk::writing_project::Output),
}
#[derive(Debug)]
pub(super) struct WireFailure {
    pub code: &'static str,
    pub message: String,
}
impl WireFailure {
    fn protocol(error: impl ToString) -> Self {
        Self {
            code: "plugin_host_protocol_invalid",
            message: error.to_string(),
        }
    }
    fn mismatch(error: impl ToString) -> Self {
        Self {
            code: "plugin_host_response_mismatch",
            message: error.to_string(),
        }
    }
}
impl WireRequest {
    pub(super) fn encode(&self) -> Result<Vec<u8>, WireFailure> {
        match self {
            Self::Writing(request) => {
                writing::encode_request(request).map_err(WireFailure::protocol)
            }
            Self::Literature(request) => {
                literature::encode_request(request).map_err(WireFailure::protocol)
            }
            Self::Projects(request) => {
                projects::encode_request(request).map_err(WireFailure::protocol)
            }
        }
    }
    pub(super) fn decode_response(&self, bytes: &[u8]) -> Result<WireResponse, WireFailure> {
        match self {
            Self::Writing(request) => {
                let response = writing::decode_response(bytes).map_err(WireFailure::protocol)?;
                response
                    .validate_for_request(request)
                    .map_err(WireFailure::mismatch)?;
                Ok(WireResponse::Writing(response.body))
            }
            Self::Literature(request) => {
                let response = literature::decode_response(bytes).map_err(WireFailure::protocol)?;
                response
                    .validate_for_request(request)
                    .map_err(WireFailure::mismatch)?;
                Ok(WireResponse::Literature(response.body))
            }
            Self::Projects(request) => {
                let response = projects::decode_response(bytes).map_err(WireFailure::protocol)?;
                response
                    .validate_for_request(request)
                    .map_err(WireFailure::mismatch)?;
                Ok(WireResponse::ProjectBody(response.body))
            }
        }
    }
}
#[cfg(test)]
mod tests;
