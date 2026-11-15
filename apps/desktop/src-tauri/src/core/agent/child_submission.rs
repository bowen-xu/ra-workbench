//! Preserve the original child submission identity when admission is uncertain.
use research_agent_core::{
    conversation::{CatalogDataFailureKind, CatalogMutationError},
    runner::{RunError, ToolOutcomeDiagnostic},
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionFailure {
    pub child_id: Uuid,
    pub request_id: Uuid,
    pub operation_id: Uuid,
    pub reason: super::catalog_commands::ArchiveCommandError,
}

pub(crate) fn uncertain(error: &CatalogMutationError) -> bool {
    match error {
        // A validation error before the receipt lookup cannot prove this child
        // was never admitted by an earlier invocation with the same identity.
        CatalogMutationError::Domain(_) => true,
        CatalogMutationError::Data(data) => matches!(
            data.kind,
            CatalogDataFailureKind::Unknown
                | CatalogDataFailureKind::Partial
                | CatalogDataFailureKind::ExternalUnknown
                | CatalogDataFailureKind::RecoveryRequired
                | CatalogDataFailureKind::Corrupt
                | CatalogDataFailureKind::Conflict
        ),
    }
}

pub(crate) fn tool_error(
    failure: &SubmissionFailure,
    child: Uuid,
    parent: Uuid,
    turn: Uuid,
    context: Uuid,
) -> RunError {
    let expected_request = research_agent_core::conversation::child_submission_request_id(child);
    let expected_operation = Uuid::new_v5(&child, b"agent.child.submission.persist:v1");
    if failure.child_id != child
        || failure.request_id != expected_request
        || failure.operation_id != expected_operation
    {
        return RunError::ToolOutcomeUnknown;
    }
    let super::catalog_commands::ArchiveCommandError::Data(data) = &failure.reason else {
        return RunError::ToolOutcomeUnknown;
    };
    if data.diagnostic.operation_id != expected_operation {
        return RunError::ToolOutcomeUnknown;
    }
    ToolOutcomeDiagnostic::try_from_value(serde_json::json!({
        "code":data.kind.code(), "phase":data.diagnostic.phase, "safeCause":data.diagnostic.safe_cause,
        "conversationId":parent, "turnId":turn, "contextSnapshotId":context,
        "resultId":child, "requestId":expected_request, "journalRequestId":expected_operation,
        "sqliteCode":data.diagnostic.sqlite_code, "sqliteExtendedCode":data.diagnostic.sqlite_extended_code,
    })).map(|detail| RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail))).unwrap_or(RunError::ToolOutcomeUnknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::conversation::{CatalogDataFailure, CatalogMutationDiagnostic};

    #[test]
    fn validation_before_receipt_lookup_does_not_authorize_cleanup() {
        use research_agent_core::conversation::CatalogError;
        assert!(uncertain(&CatalogMutationError::Domain(
            CatalogError::InvalidInput
        )));
        assert!(uncertain(&CatalogMutationError::Domain(
            CatalogError::NotFound
        )));
    }

    #[test]
    fn recovery_diagnostic_is_bound_to_the_original_child_request_and_operation() {
        let child = Uuid::new_v4();
        let request = research_agent_core::conversation::child_submission_request_id(child);
        let operation = Uuid::new_v5(&child, b"agent.child.submission.persist:v1");
        let mut failure = SubmissionFailure {
            child_id: child,
            request_id: request,
            operation_id: operation,
            reason: super::super::catalog_commands::ArchiveCommandError::Data(CatalogDataFailure {
                kind: CatalogDataFailureKind::Unknown,
                diagnostic: Box::new(CatalogMutationDiagnostic {
                    operation_id: operation,
                    epoch: None,
                    sql_fence: None,
                    phase: "commit",
                    sqlite_code: Some("ConstraintViolation".into()),
                    sqlite_extended_code: Some(787),
                    safe_cause: "sql_commit_outcome_unknown",
                }),
            }),
        };
        let diagnostic = match tool_error(
            &failure,
            child,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        ) {
            RunError::ToolOutcomeUnknownDiagnostic(value) => serde_json::to_value(value).unwrap(),
            other => panic!("missing recovery diagnostic: {other:?}"),
        };
        assert_eq!(diagnostic["requestId"], request.to_string());
        assert_eq!(diagnostic["journalRequestId"], operation.to_string());
        assert_eq!(diagnostic["sqliteExtendedCode"], 787);
        failure.request_id = Uuid::new_v4();
        assert!(matches!(
            tool_error(
                &failure,
                child,
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4()
            ),
            RunError::ToolOutcomeUnknown
        ));
    }
}
