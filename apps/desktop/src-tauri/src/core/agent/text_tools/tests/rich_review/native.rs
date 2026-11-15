use super::*;

#[tokio::test]
#[ignore = "Retains a synthetic rich-review database for the disposable development app"]
async fn native_rich_note_review_fixture() {
    let f = rich_fixture();
    {
        let db = f.tools.store.connection().unwrap();
        db.execute(
            "update note_pages set title='笔记块内 Diff 原生验收' where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)", params![f.tools.record.conversation_id.to_string(), f.page.to_string()]).unwrap();
    }
    read(&f, &path(f.page)).await;
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n-first old\n+first new\n@@\n-third old\n+third new\n*** End Patch",
        path(f.page)
    );
    let output = f
        .tools
        .authorize_and_execute(research_agent_core::model::ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "native-rich-fixture-patch".into(),
            name: PATCH.into(),
            arguments: json!({"patch":patch}),
        })
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    let destination = tempfile::Builder::new()
        .prefix("research-rich-note-native-")
        .tempdir()
        .unwrap()
        .keep();
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "vacuum into ?1",
            [destination
                .join("research-assistant.sqlite3")
                .to_str()
                .unwrap()],
        )
        .unwrap();
    let shown = review(&f);
    assert_eq!(shown.pages[0].chunks.len(), 2);
    println!(
        "NATIVE_RICH_NOTE_DIRECTORY={}\nPAGE_ID={}\nCONVERSATION_ID={}\nPROPOSAL_ID={}\nGENERATION={}",
        destination.display(),
        f.page,
        f.tools.record.conversation_id,
        shown.proposal_id,
        shown.generation
    );
}
