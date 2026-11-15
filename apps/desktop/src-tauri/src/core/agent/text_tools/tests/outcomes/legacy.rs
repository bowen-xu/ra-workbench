use super::*;

struct LegacyValidator;
impl ToolExecutor for LegacyValidator {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        panic!("the legacy file backend owns this call")
    }
}

#[tokio::test]
async fn version_one_unknown_journal_is_not_returned_as_ordinary_tool_feedback() {
    let mut fixture = fixture(true);
    fixture.tools.previous = Arc::new(LegacyValidator);
    fixture.tools.previous_names.insert(PATCH.into());
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "legacy-lost".into(),
        name: PATCH.into(),
        arguments: json!({"operations":[{"path":"unknown.txt","before":"old","after":"new"}]}),
    };
    let identity = format!("{}:{}", fixture.tools.record.request_id, call.call_id);
    {
        let db = fixture.tools.store.connection().unwrap();
        db.execute("update agent_text_tool_contracts set version=1 where conversation_id=?1 and request_id=?2",
            params![fixture.tools.record.conversation_id.to_string(), fixture.tools.record.request_id.to_string()]).unwrap();
        assert!(
            journal::insert_sql(
                &db,
                fixture.tools.record.conversation_id,
                &identity,
                &call.arguments.to_string(),
            )
            .is_ok()
        );
    }
    assert!(matches!(
        fixture.tools.authorize_and_execute(call).await,
        Err(RunError::ToolOutcomeUnknown)
    ));
    let state: String = fixture
        .tools
        .store
        .connection()
        .unwrap()
        .query_row(
            "select state from agent_text_patch_calls where conversation_id=?1 and request_id=?2",
            params![fixture.tools.record.conversation_id.to_string(), identity],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "running",
        "the original version-one receipt remains for reconciliation"
    );
}
