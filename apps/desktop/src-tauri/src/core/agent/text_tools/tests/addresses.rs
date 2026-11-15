use super::*;
use research_agent_core::model::ToolCall;

#[test]
fn unknown_agent_custom_and_general_assets_do_not_query_other_catalogs() {
    let f = fixture(true);
    // No tables: a namespace leak would report a database error instead of missing.
    let empty = rusqlite::Connection::open_in_memory().unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../../../../tests/fixtures/source-addresses.json"
    ))
    .unwrap();
    for path in fixture["unregistered"].as_array().unwrap() {
        let path = path.as_str().unwrap();
        assert!(
            matches!(super::super::addresses::resolve(&empty, &f.tools.record, path), Err(error) if error == "找不到文件对象"),
            "{path}"
        );
        assert!(
            matches!(f.tools.backend(path), Err(error) if error == "找不到文件对象"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn existing_source_addresses_keep_original_names_and_receipts_without_markdown_filenames() {
    let f = fixture(true);
    let target = path(f.page);
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='研究提纲' where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    let read = f
        .tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "source-read".into(),
            name: READ.into(),
            arguments: json!({"path":target}),
        })
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.content);
    let view: Value = serde_json::from_str(&read.content).unwrap();
    assert_eq!(view["path"], target);
    assert_eq!(view["name"], "研究提纲");
    assert!(!read.content.contains(".md"));
    assert!(read.content.contains("source://notes/"));
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "source-patch".into(),
        name: PATCH.into(),
        arguments: json!({"patch":patch(&target,"old","new")}),
    };
    let output = f.tools.authorize_and_execute(call.clone()).await.unwrap();
    let result: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["targets"][0]["path"], target);
    assert_eq!(result["targets"][0]["name"], "研究提纲");
    assert_eq!(result["targets"][0]["status"], "applied");
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='手动修改后的名称' where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    assert_eq!(
        output.content,
        f.tools.authorize_and_execute(call).await.unwrap().content
    );
}

#[tokio::test]
async fn source_parent_creation_allocates_real_identity_without_invented_filenames() {
    let f = fixture(true);
    let parent = path(f.parent);
    let add = format!(
        "*** Begin Patch\n*** Add File: {parent}\n+---\n+title: 新页面\n+---\n+\n+新正文\n*** End Patch"
    );
    let added = f.tools.apply("source-add", &add).await.unwrap();
    assert_eq!(added["targets"][0]["status"], "applied", "{added}");
    assert_eq!(added["targets"][0]["name"], "新页面");
    let shown = review(&f);
    let child = shown
        .pages
        .iter()
        .find(|page| page.kind == "create")
        .unwrap();
    assert!(child.path.starts_with("source://"));
    assert!(!child.path.ends_with(".md"));
    let view = read(&f, &child.path).await;
    assert_eq!(view["name"], "新页面");
    decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    // A new official page does not silently expand the frozen member grant.
    assert!(
        f.tools
            .read(Read {
                path: child.path.clone(),
                start_char: 0,
                max_chars: 8000,
                view: None
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_moves_preserve_identity_and_distinguish_saved_from_candidate_ownership() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    let owned = path(f.parent);
    let move_patch = format!(
        "*** Begin Patch\n*** Update File: {target}\n*** Move to: {owned}\n@@\n old\n*** End Patch"
    );
    let moved = f.tools.apply("source-move", &move_patch).await.unwrap();
    assert_eq!(moved["targets"][0]["status"], "applied", "{moved}");
    let saved = f
        .tools
        .read(Read {
            path: target.clone(),
            start_char: 0,
            max_chars: 8000,
            view: Some("saved".into()),
        })
        .await
        .unwrap();
    assert_eq!(saved["path"], target);
    assert_eq!(saved["name"], "Note");
    let proposed = read(&f, &target).await;
    assert_eq!(proposed["path"], target);
    let shown = review(&f);
    decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    read(&f, &target).await;
    let return_patch = format!(
        "*** Begin Patch\n*** Update File: {target}\n*** Move to: source://notes/\n@@\n old\n*** End Patch"
    );
    assert_eq!(
        f.tools.apply("source-return", &return_patch).await.unwrap()["targets"][0]["status"],
        "applied"
    );
    let saved = f
        .tools
        .read(Read {
            path: target,
            start_char: 0,
            max_chars: 8000,
            view: Some("saved".into()),
        })
        .await
        .unwrap();
    assert_eq!(saved["name"], "Note");
    let shown = review(&f);
    let receipt = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert!(
        receipt
            .pages
            .iter()
            .any(|page| page.page_id == f.page.to_string() && page.parent_id.is_none())
    );
}

#[tokio::test]
async fn parent_ownership_and_duplicate_note_aliases_do_not_guess_or_overwrite() {
    let f = fixture(true);
    let wrong = format!("source://notes/{}/{}", f.parent, f.page);
    assert!(
        f.tools
            .read(Read {
                path: wrong,
                start_char: 0,
                max_chars: 8000,
                view: None
            })
            .await
            .is_err()
    );
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set parent_page_id=?2 where id=?1",
            params![f.page.to_string(), f.parent.to_string()],
        )
        .unwrap();
    let root = path(f.page);
    let owned = format!("source://notes/{}/{}", f.parent, f.page);
    read(&f, &root).await;
    assert!(
        f.tools
            .read(Read {
                path: owned,
                start_char: 0,
                max_chars: 8000,
                view: None
            })
            .await
            .is_err()
    );
    let edits = format!(
        "*** Begin Patch\n*** Update File: {root}\n@@\n-old\n+first\n*** Update File: {root}\n@@\n-old\n+second\n*** End Patch"
    );
    assert!(f.tools.apply("duplicate-alias", &edits).await.is_err());
    assert!(
        decisions::review(
            &f.tools.store.connection().unwrap(),
            f.tools.record.conversation_id
        )
        .unwrap()
        .is_none()
    );
}
