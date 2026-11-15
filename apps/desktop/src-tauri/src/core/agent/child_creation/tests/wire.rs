use super::*;
use crate::agent::{
    note_child_commands, note_child_tool, reading_child_commands, reading_child_tool,
    writing_child_commands,
};
use research_agent_core::conversation::{
    CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic, CatalogMutationError,
};

fn failure(host: Host) -> CreationFailure {
    let command = command(host);
    CreationFailure {
        creation: (&command).into(),
        reason: CatalogMutationError::Data(CatalogDataFailure {
            kind: CatalogDataFailureKind::Unknown,
            diagnostic: Box::new(CatalogMutationDiagnostic {
                operation_id: command.operation_id,
                epoch: Some(Uuid::from_u128(42)),
                sql_fence: Some("9007199254740993".into()),
                phase: "commit",
                sqlite_code: Some("ConstraintViolation".into()),
                sqlite_extended_code: Some(787),
                safe_cause: "sql_commit_outcome_unknown",
            }),
        })
        .into(),
    }
}
fn assert_error(value: &Value, host: Host) {
    assert_eq!(value["code"], "catalogMutation");
    assert_eq!(restored_intent(value), command(host));
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(value["reason"]["code"], "storage");
    assert_eq!(value["reason"]["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    let detail = &value["reason"]["reason"]["reason"];
    assert_eq!(detail["sqlFence"], "9007199254740993");
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
    assert_eq!(value["creation"].as_object().unwrap().len(), 3);
    for private in ["prompt", "body", "sql", "rawResponse"] {
        assert!(value.get(private).is_none());
        assert!(value["creation"].get(private).is_none());
        assert!(detail.get(private).is_none());
    }
}

#[test]
fn three_host_error_variants_preserve_complete_creation_and_safe_diagnostic() {
    assert_error(
        &serde_json::to_value(writing_child_commands::Error::CatalogMutation(failure(
            Host::Writing,
        )))
        .unwrap(),
        Host::Writing,
    );
    assert_error(
        &serde_json::to_value(reading_child_commands::Error::CatalogMutation(failure(
            Host::Paper,
        )))
        .unwrap(),
        Host::Paper,
    );
    assert_error(
        &serde_json::to_value(note_child_commands::Error::CatalogMutation(failure(
            Host::Note,
        )))
        .unwrap(),
        Host::Note,
    );
}

#[test]
fn paper_and_note_tool_errors_preserve_recovery_intent_instead_of_a_generic_failure() {
    let paper = reading_child_tool::error_output(&reading_child_commands::Error::CatalogMutation(
        failure(Host::Paper),
    ))
    .unwrap();
    let note = note_child_tool::error_output(&note_child_commands::Error::CatalogMutation(
        failure(Host::Note),
    ))
    .unwrap();
    for (output, host) in [(paper, Host::Paper), (note, Host::Note)] {
        assert!(output.is_error);
        assert_error(&serde_json::from_str(&output.content).unwrap(), host);
    }
    let rejected =
        reading_child_tool::error_output(&reading_child_commands::Error::ParentNotRunning).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&rejected.content).unwrap(),
        json!({"code":"parentNotRunning"})
    );
}

#[test]
fn unknown_tool_creation_preserves_verified_identity_and_blocks_mismatch() {
    use research_agent_core::runner::RunError;
    for host in [Host::Paper, Host::Note] {
        let original = command(host);
        let error = failure(host);
        let result = unknown_tool_error(
            &error,
            &original,
            Uuid::from_u128(8),
            Uuid::from_u128(9),
            Uuid::from_u128(10),
        )
        .unwrap();
        let RunError::ToolOutcomeUnknownDiagnostic(detail) = result else {
            panic!("missing recovery diagnostic")
        };
        let value = serde_json::to_value(detail).unwrap();
        assert_eq!(value["requestId"], json!(original.operation_id));
        assert_eq!(value["resultId"], json!(original.id.as_uuid()));
        assert_eq!(value["conversationId"], json!(Uuid::from_u128(8)));
        assert!(value.get("binding").is_none());
        let mut mismatch = original.clone();
        mismatch.binding.resource_id.push('x');
        assert!(matches!(
            unknown_tool_error(
                &error,
                &mismatch,
                Uuid::from_u128(8),
                Uuid::from_u128(9),
                Uuid::from_u128(10)
            ),
            Some(RunError::ToolOutcomeUnknown)
        ));
    }
}
