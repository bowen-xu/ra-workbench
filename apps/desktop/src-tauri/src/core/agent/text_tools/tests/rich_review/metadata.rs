use super::*;
#[tokio::test]
async fn content_undo_projects_the_original_document_title_for_both_review_routes() {
    for whole in [false, true] {
        let f = rich_fixture();
        read(&f, &path(f.page)).await;
        let output = f
            .tools
            .apply(
                "rename",
                &patch(&path(f.page), "title: \"Note\"", "title: \"Renamed\""),
            )
            .await
            .unwrap();
        assert_eq!(output["targets"][0]["status"], "applied", "{output}");
        let shown = review(&f);
        if whole {
            decisions::decide(&f.tools.store, decision(&f, &shown, "reject")).unwrap();
        } else {
            resolve(&f, request(&f, &shown, "undo", None));
        }
        let db = f.tools.store.connection().unwrap();
        let page = note::load(&db, &f.page.to_string()).unwrap();
        assert_eq!(page.title, "Note");
        assert_eq!(page.document.title.as_deref(), Some("Note"));
        let point = crate::hbs_storage::read(&db, "note", &f.page.to_string()).unwrap();
        assert_eq!(point["saved"], point["baseline"]);
    }
}
#[tokio::test]
async fn title_review_and_its_inverse_use_document_baseline_without_a_metadata_approval() {
    let f = rich_fixture();
    read(&f, &path(f.page)).await;
    let output = f
        .tools
        .apply(
            "rename",
            &patch(&path(f.page), "title: \"Note\"", "title: \"Renamed\""),
        )
        .await
        .unwrap();
    assert_eq!(output["targets"][0]["status"], "applied", "{output}");
    let shown = review(&f);
    assert!(!shown.pages[0].metadata_pending);
    assert!(!shown.pages[0].structural_metadata_pending);
    let mut db = f.tools.store.connection().unwrap();
    let id = f.page.to_string();
    let point = crate::hbs_storage::read(&db, "note", &id).unwrap();
    assert_eq!(point["saved"]["title"], "Renamed");
    assert_eq!(point["baseline"]["title"], "Note");
    let mut write = |key: &str, at: &Value, saved: Value, baseline: Value| {
        crate::hbs_storage::test_sources::commit(
            &mut db,
            "note",
            &id,
            crate::hbs_storage::Commit {
                id: key.into(),
                expected_version: at["version"].as_i64().unwrap(),
                saved,
                baseline,
            },
        )
        .unwrap()["snapshot"]
            .clone()
    };
    let kept = write(
        "keep-title",
        &point,
        point["saved"].clone(),
        point["saved"].clone(),
    );
    let reopened = write(
        "undo-keep",
        &kept,
        kept["saved"].clone(),
        point["baseline"].clone(),
    );
    assert_eq!(reopened["saved"]["title"], "Renamed");
    assert_eq!(reopened["baseline"]["title"], "Note");
    let undone = write(
        "undo-title",
        &reopened,
        reopened["baseline"].clone(),
        reopened["baseline"].clone(),
    );
    assert_eq!(undone["saved"], undone["baseline"]);
    assert_eq!(undone["saved"]["title"], "Note");
}

#[tokio::test]
async fn manually_restoring_original_text_resolves_review_without_rejecting_or_resaving_input() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let saved = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap();
    let mut edit = request(&f, &shown, "edit", None);
    edit.document = shown.pages[0].before_document.clone();
    let restored = resolve(&f, edit);
    assert_eq!(restored.review.unwrap().state, "accept");
    assert_eq!(restored.pages[0].revision, saved.revision + 1);
    assert_eq!(
        serde_json::to_value(restored.pages[0].document.clone()).unwrap(),
        serde_json::to_value(shown.pages[0].before_document.clone().unwrap()).unwrap()
    );
}
