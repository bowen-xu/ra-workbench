//! Bounded diagnostics for the original workflow mutation and its read-back.
use super::{commands::original_error, error::AnalysisError};
use crate::{
    ai::{
        error::{AiCommandError, AiError},
        structured_resource_validation::{DecisionOrigin, Failure, FailureCause},
    },
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{
        literature_hbs::HbsOperationError,
        native::{NativeHostError, SafeDatabaseError},
    },
};
use serde_json::{Value, json};

pub(crate) fn original(error: AiError, phase: &'static str, outcome: HbsOutcome) -> Value {
    let cause = match &error {
        AiError::Database(error) => {
            FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, error)))
        }
        _ => FailureCause::Original,
    };
    let command = AiCommandError::from(error);
    let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
        failure: Failure {
            error: command.message().into(),
            cause,
        },
        phase,
        outcome,
        rollback_error: None,
    })
    .into_wire();
    let mut wire = json!(command);
    if diagnostic.is_some() || wire["nativeFailure"].is_null() {
        wire["nativeFailure"] = json!(diagnostic);
    }
    wire["analysisWorkflowPhase"] = json!(phase);
    wire["analysisWorkflowOutcome"] = json!(outcome);
    wire
}

pub(crate) fn boundary(error: BoundaryFailure<AnalysisError>) -> Value {
    let phase = error.phase;
    let outcome = error.outcome;
    let original = original_error(error.failure.error);
    let cause = match (&error.failure.cause, &original) {
        (FailureCause::Original, AiError::Database(sql)) => {
            FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(error.phase, sql)))
        }
        _ => error.failure.cause,
    };
    let command = AiCommandError::from(original);
    let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
        failure: Failure {
            error: command.message().into(),
            cause,
        },
        phase: error.phase,
        outcome: error.outcome,
        rollback_error: error.rollback_error,
    })
    .into_wire();
    let mut wire = json!(command);
    if diagnostic.is_some() || wire["nativeFailure"].is_null() {
        wire["nativeFailure"] = json!(diagnostic);
    }
    wire["analysisWorkflowPhase"] = json!(phase);
    wire["analysisWorkflowOutcome"] = json!(outcome);
    wire
}

pub(crate) fn identify(mut error: Value, workflow: &str, package: &str) -> Value {
    error["workflowId"] = json!(workflow);
    error["analysisWorkflowOperation"] = json!("failAttempt");
    if error["nativeFailure"].is_object() && error["nativeFailure"]["packageKey"].is_null() {
        error["nativeFailure"]["packageKey"] = json!(package);
    }
    if error["analysisWorkflowPhase"].is_null() && error["nativeFailure"]["phase"].is_string() {
        error["analysisWorkflowPhase"] = error["nativeFailure"]["phase"].clone();
    }
    if error["analysisWorkflowOutcome"].is_null() && error["nativeFailure"]["outcome"].is_string() {
        error["analysisWorkflowOutcome"] = error["nativeFailure"]["outcome"].clone();
    }
    error
}

pub(crate) fn native(
    error: Box<NativeHostError>,
    phase: &'static str,
    outcome: HbsOutcome,
) -> Value {
    let code = error.code.clone();
    let message = error.message.clone();
    let (_, diagnostic) = HbsOperationError::native(error, phase, outcome).into_wire();
    json!({"code":code,"message":message,"nativeFailure":diagnostic})
}

pub(crate) fn rejected(message: String, origin: DecisionOrigin, phase: &'static str) -> Value {
    let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
        failure: Failure {
            error: message.clone(),
            cause: FailureCause::NativeRejection(Box::new(origin)),
        },
        phase,
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
    .into_wire();
    json!({"code":"invalid_input","message":message,"nativeFailure":diagnostic})
}

pub(crate) fn annotate(
    mut error: Value,
    workflow: &str,
    phase: &'static str,
    outcome: HbsOutcome,
) -> Value {
    error["workflowId"] = json!(workflow);
    error["analysisWorkflowOperation"] = json!("failAttempt");
    error["analysisWorkflowPhase"] = json!(phase);
    error["analysisWorkflowOutcome"] = json!(outcome);
    if error["nativeFailure"].is_object() {
        error["nativeFailure"]["phase"] = json!(phase);
        error["nativeFailure"]["outcome"] = json!(outcome);
    }
    error
}
