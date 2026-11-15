use super::*;

#[tokio::test]
#[ignore = "Retains a synthetic database for disposable native application review/undo verification"]
async fn native_text_patch_review_fixture() {
    let f = fixture(true);
    let target = path(f.page);
    {
        let db = f.tools.store.connection().unwrap();
        db.execute(
            "update note_pages set title='统一补丁原生验收' where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)", params![f.tools.record.conversation_id.to_string(), f.page.to_string()]).unwrap();
    }
    read(&f, &target).await;
    let result = f.tools.authorize_and_execute(research_agent_core::model::ToolCall {
namespace: None,
raw_arguments_json: None, call_id:"native-fixture-patch".into(), name:PATCH.into(), arguments:json!({"patch":patch(&target,"old","new\n\n## 工具生成的段落\n\n候选只在接受后保存。")}) }).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    let root = tempfile::Builder::new()
        .prefix("research-text-patch-native-")
        .tempdir()
        .unwrap()
        .keep();
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "vacuum into ?1",
            [root.join("research-assistant.sqlite3").to_str().unwrap()],
        )
        .unwrap();
    println!(
        "NATIVE_TEXT_PATCH_DIRECTORY={}\nPAGE_ID={}",
        root.display(),
        f.page
    );
}

#[tokio::test]
async fn page_moves_preserve_identity_and_can_return_to_root() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    let moved = path(f.parent);
    let patch = format!(
        "*** Begin Patch\n*** Update File: {target}\n*** Move to: {moved}\n@@\n old\n*** End Patch"
    );
    let result = f.tools.apply("move", &patch).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    assert_eq!(result["targets"][0]["path"], target);
    let shown = review(&f);
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert!(
        accepted
            .pages
            .iter()
            .any(|page| page.page_id == f.page.to_string()
                && page.parent_id == Some(f.parent.to_string()))
    );
    read(&f, &target).await;
    let result = f
        .tools
        .apply(
            "return",
            &format!("*** Begin Patch\n*** Update File: {target}\n*** Move to: source://notes/\n@@\n old\n*** End Patch"),
        )
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    let shown = review(&f);
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert!(
        accepted
            .pages
            .iter()
            .any(|page| page.page_id == f.page.to_string() && page.parent_id.is_none())
    );
}

#[tokio::test]
async fn complete_blocks_move_across_pages_with_sidecars_and_group_undo() {
    let f = fixture(true);
    let source = path(f.page);
    let destination = path(f.parent);
    let original = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap();
    read(&f, &source).await;
    read(&f, &destination).await;
    let patch = format!(
        "*** Begin Patch\n*** Update File: {source}\n@@\n-old\n*** Update File: {destination}\n@@\n parent\n+\n+old\n*** End Patch"
    );
    let result = f.tools.apply("block-move", &patch).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    let shown = review(&f);
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    let moved = accepted
        .pages
        .iter()
        .find(|page| page.page_id == f.parent.to_string())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&moved.document.blocks[1]).unwrap(),
        serde_json::to_value(&original.document.blocks[0]).unwrap()
    );
    let undo = decisions::decide(&f.tools.store, decision(&f, &shown, "undo")).unwrap();
    assert_eq!(
        serde_json::to_value(
            &undo
                .pages
                .iter()
                .find(|page| page.page_id == original.page_id)
                .unwrap()
                .document
        )
        .unwrap(),
        serde_json::to_value(original.document).unwrap()
    );
}

#[tokio::test]
async fn whole_undo_rolls_back_all_documents_history_and_receipt_on_storage_failure() {
    let f = fixture(true);
    let source = path(f.page);
    let destination = path(f.parent);
    read(&f, &source).await;
    read(&f, &destination).await;
    let patch = format!(
        "*** Begin Patch\n*** Update File: {source}\n@@\n-old\n+changed\n*** Update File: {destination}\n@@\n-parent\n+changed parent\n*** End Patch"
    );
    f.tools.apply("atomic", &patch).await.unwrap();
    let shown = review(&f);
    let request = decision(&f, &shown, "reject");
    let identity = request.request_id;
    let before = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap();
    f.tools.store.connection().unwrap().execute_batch("create trigger fail_patch before update on note_pages when new.revision > old.revision begin select raise(abort,'injected failure'); end;").unwrap();
    assert!(decisions::decide(&f.tools.store, request).is_err());
    let db = f.tools.store.connection().unwrap();
    assert_eq!(
        serde_json::to_value(note::load(&db, &f.page.to_string()).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert!(
        !db.query_row(
            "select exists(select 1 from agent_note_patch_decisions where request_id=?1)",
            [identity.to_string()],
            |row| row.get::<_, bool>(0)
        )
        .unwrap()
    );
    drop(db);
    assert_eq!(review(&f).state, "pending");
}

#[tokio::test]
async fn newer_submission_cannot_borrow_reads_and_revoked_write_prevents_acceptance() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    f.tools
        .apply("shared-call", &patch(&target, "old", "new"))
        .await
        .unwrap();
    let shown = review(&f);
    let mut input = f.tools.record.input.clone();
    for source in &mut input.context {
        let mut anchor: Value = serde_json::from_str(source.anchor.as_ref().unwrap()).unwrap();
        anchor["contextAccess"] = json!("readOnly");
        source.anchor = Some(anchor.to_string());
    }
    let record = {
        let mut db = f.tools.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let record =
            submissions::insert_in(&tx, f.tools.record.conversation_id, Uuid::new_v4(), input)
                .unwrap();
        tx.commit().unwrap();
        record
    };
    let next = TextTools {
        store: f.tools.store.clone(),
        record,
        previous: Arc::new(RejectingToolExecutor),
        previous_names: Default::default(),
        previous_qualified_names: Default::default(),
        delivery_attempted: Default::default(),
        delivery_sealed: Default::default(),
        delivery_lease: Default::default(),
        cancellation: Default::default(),
        #[cfg(target_os = "macos")]
        file_workspace: None,
    };
    let result = next
        .apply("shared-call", &patch(&target, "new", "newer"))
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "failed");
    assert!(decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).is_err());
}
