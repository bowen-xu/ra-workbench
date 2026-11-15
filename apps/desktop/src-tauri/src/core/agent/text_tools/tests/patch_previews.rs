use super::*;

#[tokio::test]
async fn applied_preview_is_frozen_in_the_original_receipt_and_failed_edits_have_none() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    let input = patch(&target, "old", "new");
    let first = f.tools.apply("display-diff", &input).await.unwrap();
    assert_eq!(first["fileDiffs"][0]["path"], target);
    assert!(
        !first["fileDiffs"][0]["hunks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.tools
        .apply("later-edit", &patch(&target, "new", "later"))
        .await
        .unwrap();
    assert_eq!(f.tools.apply("display-diff", &input).await.unwrap(), first);
    let failed = f
        .tools
        .apply("failed-display", &patch(&target, "missing text", "other"))
        .await
        .unwrap();
    assert!(failed.get("fileDiffs").is_none());
}

#[test]
fn preview_uses_prepared_unicode_positions_and_bounds_large_deletions() {
    let source = "one\r\n一🧪\r\nold\r\nlast\r\n";
    let hunk = text_patch::parse(
        "*** Begin Patch\n*** Update File: test.ts\n@@\n-old\n+new\n+extra\n*** End Patch",
    )
    .unwrap()
    .remove(0);
    let change = text_patch::backend::prepare(
        &hunk,
        Some(TextSnapshot {
            path: "test.ts".into(),
            name: Some("test.ts".into()),
            text: source.into(),
            version: "original".into(),
        }),
    )
    .unwrap();
    let preview = patch_preview::preview(&change);
    assert_eq!(preview["hunks"][0]["oldStart"], 1);
    assert_eq!(preview["hunks"][0]["newStart"], 1);
    assert_eq!(
        preview["hunks"][0]["lines"],
        json!([" one", " 一🧪", "-old", "+new", "+extra", " last"])
    );
    let large = TextChange {
        path: "large.md".into(),
        destination: None,
        before: Some(TextSnapshot {
            path: "large.md".into(),
            name: None,
            text: "line\n".repeat(700),
            version: "original".into(),
        }),
        after: None,
        edits: vec![],
    };
    let preview = patch_preview::preview(&large);
    assert_eq!(preview["omitted"], true);
    assert_eq!(preview["hunks"], json!([]));
}
#[test]
fn saved_previews_override_repair_drafts_and_remain_bounded_on_recovery() {
    let path = "source://synthetic-resource";
    let after = json!({"saved":"synthetic formal content"});
    let saved = patch_preview::saved_structured(path, "Synthetic", None, Some(&after));
    let result = TargetResult {
        path: path.into(),
        name: Some("Synthetic".into()),
        status: TargetStatus::Applied,
        message: json!({"savedDiff":saved}).to_string(),
        change_id: Some("synthetic-change".into()),
        generation: None,
    };
    let wrong = json!({"path":path,"hunks":[{"lines":["-invalid draft"]}]});
    let shown = patch_preview::applied(&[wrong], std::slice::from_ref(&result));
    assert_eq!(shown[0]["comparison"], "saved");
    assert_eq!(shown[0]["afterSaved"], after);
    assert!(shown[0]["beforeSaved"].is_null());
    assert_eq!(
        patch_preview::applied(&[], std::slice::from_ref(&result)),
        shown
    );
    let refused = TargetResult {
        status: TargetStatus::Failed,
        ..result
    };
    assert!(patch_preview::applied(&[], &[refused]).is_empty());
    let large = json!({"lines":vec!["large line";700]});
    let omitted = patch_preview::saved_structured(path, "Synthetic", None, Some(&large));
    assert_eq!(omitted["omitted"], true);
    assert!(omitted.get("afterSaved").is_none());
    assert!(omitted.get("beforeSaved").is_none());
}
