use super::*;

#[tokio::test]
async fn patches_with_source_citations_save_reopen_and_keep_without_granting_target_access() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    // The linked source need not be writable or included in the note's write grant.
    let href = format!(
        "source://library/{}/{}:#line{{3:12-4:14-20}}",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    let body = format!("## Algorithm\n\nSource: [Paper](<{href}>)");
    let result = f
        .tools
        .apply("source-citation", &patch(&target, "old", &body))
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    // Load using a new SQLite connection, independently of tool/editor memory.
    let reopened = Connection::open(f._root.path().join("fixture.sqlite3")).unwrap();
    let saved = crate::notes::load_document_value(&reopened, &f.page.to_string()).unwrap();
    assert_eq!(saved.blocks[1].content["items"][1]["href"], href);
    let view = read(&f, &target).await;
    assert!(view["text"].as_str().unwrap().contains(&href));
    let shown = review(&f);
    decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    let kept = crate::notes::load_document_value(&reopened, &f.page.to_string()).unwrap();
    assert_eq!(kept.blocks[1].content["items"][1]["href"], href);
}

#[tokio::test]
async fn unsafe_citations_fail_without_writing_or_creating_pending_changes() {
    for href in [
        "javascript:alert%281%29",
        "data:text/html,unsafe",
        "file:///tmp/unsafe",
    ] {
        let f = fixture(true);
        let target = path(f.page);
        let before = read(&f, &target).await;
        let result = f
            .tools
            .apply(
                "unsafe-citation",
                &patch(&target, "old", &format!("[Source]({href})")),
            )
            .await
            .unwrap();
        assert_eq!(result["targets"][0]["status"], "failed", "{result}");
        assert_eq!(read(&f, &target).await["text"], before["text"]);
        assert!(
            decisions::review(
                &f.tools.store.connection().unwrap(),
                f.tools.record.conversation_id
            )
            .unwrap()
            .is_none()
        );
    }
}
