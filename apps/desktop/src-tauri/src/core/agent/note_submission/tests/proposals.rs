use super::*;
use crate::agent::note_tools::NoteTools;
use research_agent_core::{model::ToolCall, runner::ToolExecutor};
use serde_json::{Value, json};

fn call(record: &SubmissionRecord) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "rewrite-1".into(),
        name: "research_propose_note_rewrite".into(),
        arguments: json!({
            "sourceId":record.input.context[0].source_id,"edits":[{"blockId":"selected","content":{"kind":"inline","items":[{"type":"text","text":"Revised synthetic evidence","styles":[]}]}}]
        }),
    }
}

#[tokio::test]
async fn oversized_before_content_is_rejected_instead_of_truncating_the_proposal() {
    let f = fixture(true);
    let content =
        json!({"kind":"inline","items":[{"type":"text","text":"x".repeat(128*1024),"styles":[]}]})
            .to_string();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_blocks set content_json=?1 where id='selected'",
            [&content],
        )
        .unwrap();
    let id = Uuid::new_v4();
    let record = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let tool_call = call(&record);
    let tools = NoteTools {
        store: f.store.clone(),
        receipt: receipt(&f, id),
        record,
    };
    assert!(tools.validate(&tool_call).is_err());
    assert!(tools.authorize_and_execute(tool_call).await.is_err());
    let actual: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select content_json from note_blocks where id='selected'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actual, content);
}

#[tokio::test]
async fn rewrite_tool_uses_original_snapshot_and_never_saves_the_note() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    let record = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let tool_call = call(&record);
    let tools = NoteTools {
        store: f.store.clone(),
        receipt: receipt(&f, id),
        record,
    };
    f.store
        .connection()
        .unwrap()
        .execute("update note_pages set revision=1", [])
        .unwrap();
    let changes = f.store.connection().unwrap().total_changes();
    tools.validate(&tool_call).unwrap();
    let output = tools
        .authorize_and_execute(tool_call.clone())
        .await
        .unwrap();
    assert!(!output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(value["status"], "proposed");
    assert_eq!(
        value["edits"][0]["before"]["items"][0]["text"],
        "Frozen note evidence"
    );
    assert_eq!(
        value["edits"][0]["after"]["items"][0]["text"],
        "Revised synthetic evidence"
    );
    assert_eq!(value["revision"], tools.record.input.context[0].revision);
    assert_eq!(
        output.content,
        tools
            .authorize_and_execute(tool_call.clone())
            .await
            .unwrap()
            .content
    );
    let mut another_call = tool_call;
    another_call.call_id = "rewrite-2".into();
    let other: Value = serde_json::from_str(
        &tools
            .authorize_and_execute(another_call)
            .await
            .unwrap()
            .content,
    )
    .unwrap();
    assert_ne!(value["proposalId"], other["proposalId"]);
    assert_eq!(f.store.connection().unwrap().total_changes(), changes);
}

#[tokio::test]
async fn rewrite_rechecks_binding_and_rejects_unknown_tools_fields_and_scope() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    let record = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let valid = call(&record);
    let tools = NoteTools {
        store: f.store.clone(),
        receipt: receipt(&f, id),
        record,
    };
    for (key, value) in [
        ("sourceId", json!("other")),
        ("pageId", json!(f.page)),
        ("edits", json!([])),
    ] {
        let mut invalid = valid.clone();
        invalid.arguments[key] = value;
        assert!(tools.validate(&invalid).is_err());
    }
    let mut invalid = valid.clone();
    invalid.name = "save_note_document".into();
    assert!(tools.validate(&invalid).is_err());
    let mut invalid = valid.clone();
    invalid.arguments["edits"][0]["children"] = json!([]);
    assert!(tools.validate(&invalid).is_err());
    let mut invalid = valid.clone();
    invalid.arguments["edits"][0]["content"]["items"][0]["text"] = json!("x".repeat(32 * 1024));
    assert!(tools.validate(&invalid).is_err());
    tools.validate(&valid).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(tools.authorize_and_execute(valid).await.is_err());
}
