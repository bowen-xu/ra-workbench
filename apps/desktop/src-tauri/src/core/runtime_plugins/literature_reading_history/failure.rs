use crate::{
    ai::{
        error::{AiCommandError, AiError},
        structured_resource_validation::{DecisionOrigin, FailureCause},
    },
    runtime_plugins::{
        literature_context::ContextFailure,
        native::{NativeHostError, NativeProfile},
    },
};
use serde_json::{Value, json};

pub(crate) fn native_wire(error: Box<NativeHostError>) -> Value {
    json!(AiCommandError::from(AiError::NativeWorkflow {
        code: "native_workflow_failed",
        failure: error
    }))
}

pub(crate) fn worker_failure(may_have_written: bool) -> ContextFailure {
    let mut failure =
        crate::runtime_plugins::literature_context::source_failure(AiError::NativeWorkflow {
            code: "native_workflow_failed",
            failure: NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_source_worker_unsettled",
                "旧阅读历史本地操作未确认完成，请核对原会话后重试。",
            ),
        });
    if may_have_written {
        failure.outcome = crate::hbs_storage::boundary::HbsOutcome::Unknown;
    }
    failure
}
pub(crate) fn source_wire(
    boundary: crate::hbs_storage::boundary::BoundaryFailure<AiError>,
    key: &str,
    origin: Option<&DecisionOrigin>,
) -> Value {
    let unknown = boundary.needs_recovery();
    let mut error = boundary.failure.error;
    if let FailureCause::SourceData(cause) = &boundary.failure.cause
        && cause.original_sql_failure.is_some()
    {
        error = AiError::PluginDatabase(cause.clone());
    }
    let mut failure = match error {
        AiError::NativeWorkflow { failure, .. } => failure,
        error => {
            let command = AiCommandError::from(error);
            NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                command.code(),
                command.message(),
            )
        }
    };
    failure.package_key = Some(key.into());
    match boundary.failure.cause {
        FailureCause::SourceData(cause) => failure.data_error = Some(cause),
        FailureCause::SourceDatabase(cause) => failure.save_database_error = Some(cause),
        _ => {}
    }
    if let Some(origin) = origin {
        failure.instance_id = Some(origin.instance_id.clone());
        failure.request_id = Some(origin.native_request_id.clone());
    }
    if unknown {
        failure.code = "data_operation_outcome_unknown".into();
        failure.message = format!(
            "旧阅读历史操作尚未确认，请重开核对原会话。{}",
            failure.message
        );
        failure.save_outcome =
            Some(crate::runtime_plugins::literature_catalog::save::SaveOutcome::Unknown);
    }
    native_wire(failure)
}
