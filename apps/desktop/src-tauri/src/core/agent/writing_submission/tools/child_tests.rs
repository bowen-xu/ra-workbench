use super::*;

#[test]
fn unknown_child_creation_stops_with_original_writing_identity() {
    use crate::agent::{
        catalog_commands::CatalogMutationCommandError,
        child_creation,
        repository::{SqliteConversationCatalog, SqliteConversationStore},
        writing_child_commands::Error,
    };
    use research_agent_core::{
        conversation::{
            BindingKind, CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic,
            ConversationBinding,
        },
        runner::RunError,
    };
    let f = super::super::tests::fixture(true);
    let turn = Uuid::new_v4();
    let context = ChildSpawnContext {
        store: f.store.clone(),
        catalog: SqliteConversationCatalog::from_app_store(&f.store),
        service: WritingSubmissionService::from_app_store(&f.store),
        models: f.models.clone(),
        sessions: Arc::new(AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into())
                .unwrap(),
        ))),
        parent_conversation_id: f.conversation,
        parent_turn_id: turn,
        context_snapshot_id: Uuid::new_v4(),
        manuscript_id: f.manuscript,
        approved_servers: HashSet::new(),
    };
    let child = child_creation::writing_tool_child_id(turn, "call-child");
    let intent = child_creation::intent(
        child,
        ConversationBinding {
            kind: BindingKind::Manuscript,
            resource_id: f.manuscript.to_string(),
        },
        child_creation::Host::Writing,
    );
    for kind in [
        CatalogDataFailureKind::Unknown,
        CatalogDataFailureKind::Partial,
        CatalogDataFailureKind::ExternalUnknown,
        CatalogDataFailureKind::RecoveryRequired,
    ] {
        let error = Error::CatalogMutation(child_creation::CreationFailure {
            creation: (&intent).into(),
            reason: CatalogMutationCommandError::Data(CatalogDataFailure {
                kind,
                diagnostic: Box::new(CatalogMutationDiagnostic {
                    operation_id: intent.operation_id,
                    epoch: None,
                    sql_fence: None,
                    phase: "commit",
                    sqlite_code: Some("ConstraintViolation".into()),
                    sqlite_extended_code: Some(787),
                    safe_cause: "sql_commit_outcome_unknown",
                }),
            }),
        });
        let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) =
            writing_child_failure(&context, child, &error)
        else {
            panic!("unknown must stop the runner")
        };
        let value = serde_json::to_value(detail).unwrap();
        assert_eq!(value["requestId"], json!(intent.operation_id));
        assert_eq!(value["resultId"], json!(child));
        assert_eq!(value["manuscriptId"], json!(f.manuscript));
        assert_eq!(value["turnId"], json!(turn));
        assert_eq!(value["sqliteExtendedCode"], 787);
    }
    assert!(
        writing_child_failure(&context, child, &Error::ParentNotRunning)
            .unwrap()
            .is_error
    );
}

fn call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "call-child".into(),
        name: SPAWN_SUBTASK.into(),
        arguments,
    }
}

#[test]
fn child_tool_schema_is_bounded_and_strict() {
    let definition = ChildSpawnTool::definition();
    assert_eq!(definition.name, SPAWN_SUBTASK);
    assert_eq!(definition.parameters["additionalProperties"], false);
    assert_eq!(
        definition.parameters["properties"]["prompt"]["maxLength"],
        16384
    );
}

#[test]
fn child_tool_rejects_empty_or_unknown_arguments() {
    assert!(
        ChildSpawnTool::parse(
            &call(json!({"prompt": " "})),
            Uuid::new_v4(),
            &HashSet::new(),
        )
        .is_err()
    );
    assert!(
        ChildSpawnTool::parse(
            &call(json!({"prompt": "delegate", "extra": true})),
            Uuid::new_v4(),
            &HashSet::new(),
        )
        .is_err()
    );
}

#[test]
fn child_tool_accepts_a_bounded_prompt() {
    let (parsed, _request) = ChildSpawnTool::parse(
        &call(json!({"prompt": "compare two related methods"})),
        Uuid::new_v4(),
        &HashSet::new(),
    )
    .expect("bounded child prompt");
    assert_eq!(parsed.prompt, "compare two related methods");
}

#[test]
fn child_tool_validation_preserves_parent_mcp_scope() {
    let approved = HashSet::from(["literature".to_owned()]);
    let (parsed, request) = ChildSpawnTool::parse(
        &call(json!({"prompt": "use the approved literature tool"})),
        Uuid::new_v4(),
        &approved,
    )
    .expect("approved child prompt");
    assert_eq!(parsed.prompt, "use the approved literature tool");
    assert_eq!(request.approved_servers, approved);
}
