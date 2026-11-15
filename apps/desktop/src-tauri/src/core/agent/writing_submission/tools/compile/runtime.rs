//! Runtime selection at the same authorized formal feedback boundary.
use super::*;
use crate::{
    data_interface::DatabaseGuard, runtime_plugins::writing_projects::compile::feedback::Feedback,
};

fn unavailable(
    error: Box<crate::runtime_plugins::native::NativeHostError>,
    prefix: &str,
) -> Result<ToolOutput, RunError> {
    // The existing closed diagnostic retains actual host/request identities and
    // safe cleanup causes. Display alone intentionally omits those fields.
    let diagnostic = serde_json::to_string(&error).map_err(|_| RunError::EventSink)?;
    Ok(ToolOutput {
        content: format!("{prefix}: {diagnostic}"),
        is_error: true,
    })
}

pub(super) fn read(
    store: &AppStore,
    db: DatabaseGuard<'_>,
    manuscript_id: Uuid,
    request: ParsedRequest,
    runtime: tokio::runtime::Handle,
) -> Result<ToolOutput, RunError> {
    let binding = store
        .native_writing_context()
        .cloned()
        .ok_or(RunError::ToolRejected)?;
    // Resource admission only reads the selected package. No Native call or
    // retained database owner enters this adapter's constructor.
    let source = match Feedback::new(binding, runtime) {
        Ok(source) => source,
        Err(error) => return unavailable(error, "Compile feedback unavailable"),
    };
    let run = match &request {
        ParsedRequest::Diagnostics(_) => source.latest(&db, &manuscript_id.to_string()),
        ParsedRequest::Forward(_) => source.latest_pdf(&db, &manuscript_id.to_string()),
    }
    .map_err(|_| RunError::EventSink)?;
    drop(db);
    match request {
        ParsedRequest::Diagnostics(request) => {
            let Some(run) = run else {
                return Ok(render(None, request.include_raw_output));
            };
            let _operation = match source.enter() {
                Ok(operation) => operation,
                Err(error) => return unavailable(error, "Compile feedback unavailable"),
            };
            match source.project(run) {
                Ok(run) => Ok(render(Some(run), request.include_raw_output)),
                Err(error) => unavailable(error, "Compile feedback unavailable"),
            }
        }
        ParsedRequest::Forward(request) => {
            let run = run.ok_or(RunError::ToolRejected)?;
            let _operation = match source.enter() {
                Ok(operation) => operation,
                Err(error) => return unavailable(error, "SyncTeX unavailable"),
            };
            match source.forward(
                store,
                &run.id,
                &request.relative_path,
                request.line,
                request.column,
            ) {
                Ok(locations) => Ok(ToolOutput {
                    content: serde_json::to_string(&locations).map_err(|_| RunError::EventSink)?,
                    is_error: false,
                }),
                Err(error) => unavailable(error, "SyncTeX unavailable"),
            }
        }
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
