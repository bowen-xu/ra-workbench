//! Pure canonical decisions drive the same Core capture for independent-host comparisons.
use super::*;
use rusqlite::Connection;

pub(crate) fn preview(
    db: &Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> Result<AiTaskPreview, AiError> {
    let mut policy = OverviewPolicy::default();
    loop {
        match policy.capture(db, credentials, draft) {
            Ok(value) => return Ok(value),
            Err(CaptureError::Original(error)) => return Err(*error),
            Err(CaptureError::Need(input)) => {
                let output = research_literature_library_native::catalog::dispatch(
                    CatalogInput::DecideAnalysisWorkflow {
                        input: Box::new(WorkflowWireInputV1 {
                            cbor_base64: encode_payload(&*input)
                                .map_err(|_| AiError::TemporaryUnavailable)?,
                        }),
                    },
                )
                .map_err(|_| AiError::TemporaryUnavailable)?;
                let CatalogOutput::AnalysisWorkflow { outcome } = output else {
                    return Err(AiError::TemporaryUnavailable);
                };
                let decision = decode_payload(&outcome.decision_cbor_base64)
                    .map_err(|_| AiError::TemporaryUnavailable)?;
                policy.record(
                    *input,
                    decision,
                    DecisionOrigin {
                        package_key: key.into(),
                        instance_id: "canonical-pure-fixture".into(),
                        native_request_id: uuid::Uuid::new_v4().to_string(),
                    },
                );
            }
            Err(CaptureError::MatrixResources) => {
                let package =
                    package::accept_analysis_workflow(catalog, key).map_err(template::technical)?;
                policy.matrix_sql = Some(
                    crate::runtime_plugins::literature_matrix::context::MatrixContextSql::load(
                        catalog, &package,
                    )
                    .map_err(|_| AiError::TemporaryUnavailable)?,
                );
            }
        }
    }
}
