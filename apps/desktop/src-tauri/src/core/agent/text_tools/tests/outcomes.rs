use super::*;
#[path = "outcomes/legacy.rs"]
mod legacy;

#[tokio::test]
async fn owned_unknown_patch_result_keeps_original_journal_and_requires_reconciliation() {
    let fixture = fixture(true);
    let target = path(fixture.page);
    read(&fixture, &target).await;
    let patch = patch(&target, "old", "new");
    let identity = format!("{}:lost", fixture.tools.record.request_id);
    {
        let db = fixture.tools.store.connection().unwrap();
        assert!(
            journal::insert_sql(&db, fixture.tools.record.conversation_id, &identity, &patch)
                .is_ok()
        );
    }
    let result = fixture
        .tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "lost".into(),
            name: PATCH.into(),
            arguments: json!({"patch":patch}),
        })
        .await;
    let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) = result else {
        panic!("missing original receipt must retain its safe recovery diagnostic");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(detail["code"], "TEXT_PATCH_JOURNAL_UNCONFIRMED");
    assert_eq!(detail["phase"], "target-query");
    assert_eq!(detail["safeCause"], "target_receipt_unavailable");
    assert_eq!(
        detail["conversationId"],
        json!(fixture.tools.record.conversation_id)
    );
    assert_eq!(detail["requestId"], json!(fixture.tools.record.request_id));
    assert_eq!(detail["callId"], "lost");
    assert_eq!(detail["callRequestId"], identity);
    assert_eq!(detail["targetRequestId"], format!("{identity}:notes"));
    assert!(
        read(&fixture, &target).await["text"]
            .as_str()
            .unwrap()
            .contains("old")
    );
    let db = fixture.tools.store.connection().unwrap();
    let (state, value): (String, String) = db.query_row(
        "select state,results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2",
        params![fixture.tools.record.conversation_id.to_string(), identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(state, "running");
    let value: Value = serde_json::from_str(&value).unwrap();
    let targets: Vec<TargetResult> = serde_json::from_value(value["targets"].clone()).unwrap();
    assert!(
        targets
            .iter()
            .any(|target| target.status == TargetStatus::Unknown)
    );
}

#[tokio::test]
async fn known_partial_patch_result_remains_feedback_without_an_unknown_effect() {
    let fixture = fixture(true);
    let target = path(fixture.page);
    read(&fixture, &target).await;
    let patch = format!(
        "*** Begin Patch\n*** Add File: workspace/unregistered/file.txt\n+text\n*** Update File: {target}\n@@\n-old\n+new\n*** End Patch"
    );
    let output = fixture
        .tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "known-partial".into(),
            name: PATCH.into(),
            arguments: json!({"patch":patch}),
        })
        .await
        .unwrap();
    assert!(output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    let targets: Vec<TargetResult> = serde_json::from_value(value["targets"].clone()).unwrap();
    assert!(
        targets
            .iter()
            .any(|target| target.status == TargetStatus::Applied)
    );
    assert!(
        !targets
            .iter()
            .any(|target| target.status == TargetStatus::Unknown)
    );
}
