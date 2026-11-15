use super::*;

#[tokio::test]
async fn user_metadata_advances_saved_review_version_without_accepting_body_diff() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let mut db = f.tools.store.connection().unwrap();
    let before = note::load(&db, &f.page.to_string()).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "update note_pages set title='Manual title',is_favorite=1,revision=revision+1 where id=?1",
        [f.page.to_string()],
    )
    .unwrap();
    let after = note::load(&tx, &f.page.to_string()).unwrap();
    note_review::synchronize_metadata(&tx, &before, &after).unwrap();
    tx.commit().unwrap();
    drop(db);
    let updated = review(&f);
    assert!(updated.baseline_matches);
    assert_eq!(updated.generation, shown.generation);
    assert_eq!(updated.pages[0].chunks.len(), 2);
    assert_eq!(updated.pages[0].title, "Manual title");
    let mut edit = request(&f, &updated, "edit", None);
    let mut document = updated.pages[0].after_document.clone().unwrap();
    document.blocks[1] = block("middle", "manual after metadata");
    edit.document = Some(document);
    let saved = resolve(&f, edit);
    assert!(saved.review.as_ref().unwrap().baseline_matches);
    assert_eq!(saved.review.as_ref().unwrap().pages[0].chunks.len(), 2);
}

#[tokio::test]
async fn metadata_does_not_adopt_an_unrelated_external_version() {
    let f = rich_fixture();
    two_changes(&f).await;
    let mut db = f.tools.store.connection().unwrap();
    db.execute(
        "update note_pages set revision=revision+1 where id=?1",
        [f.page.to_string()],
    )
    .unwrap();
    let before = note::load(&db, &f.page.to_string()).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "update note_pages set is_favorite=1,revision=revision+1 where id=?1",
        [f.page.to_string()],
    )
    .unwrap();
    let after = note::load(&tx, &f.page.to_string()).unwrap();
    note_review::synchronize_metadata(&tx, &before, &after).unwrap();
    tx.commit().unwrap();
    drop(db);
    assert!(!review(&f).baseline_matches);
}

#[tokio::test]
async fn pending_body_is_saved_and_partial_keep_does_not_advance_its_formal_revision() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    assert_eq!(shown.persistence_version, 2);
    let saved = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap();
    assert_eq!(body(&saved.document, 0), "first new");
    assert_eq!(
        body(shown.pages[0].review_document.as_ref().unwrap(), 0),
        "first old"
    );
    let keep = resolve(&f, request(&f, &shown, "keep", Some(0)));
    assert!(keep.pages.is_empty());
    assert_eq!(
        note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
            .unwrap()
            .revision,
        saved.revision
    );
    let mut edit = request(&f, keep.review.as_ref().unwrap(), "edit", None);
    let mut document = keep.review.as_ref().unwrap().pages[0]
        .after_document
        .clone()
        .unwrap();
    document.blocks[1] = block("middle", "manual middle");
    edit.document = Some(document);
    let edited = resolve(&f, edit.clone());
    assert_eq!(edited.review.as_ref().unwrap().pages[0].chunks.len(), 1);
    assert_eq!(body(&edited.pages[0].document, 1), "manual middle");
    assert_eq!(edited.pages[0].revision, saved.revision + 1);
    assert_eq!(resolve(&f, edit).pages[0].revision, saved.revision + 1);
}

#[tokio::test]
async fn manual_saves_after_keep_preserve_the_native_baseline_history_proof() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let keep = request(&f, &shown, "keep", None);
    let accepted = resolve(&f, keep.clone());
    let initial = accepted.review.as_ref().unwrap().pages[0]
        .after_document
        .clone()
        .unwrap();
    let mut edited = initial.clone();
    edited.blocks[1] = block("middle", "manual middle");
    let mut edit = request(&f, accepted.review.as_ref().unwrap(), "edit", None);
    edit.document = Some(edited);
    let changed = resolve(&f, edit);
    assert_eq!(changed.review.as_ref().unwrap().state, "accept");
    let mut edit_undo = request(&f, changed.review.as_ref().unwrap(), "edit", None);
    edit_undo.document = Some(initial);
    let unchanged = resolve(&f, edit_undo);
    let revision = unchanged.pages[0].revision;
    let mut history = request(&f, unchanged.review.as_ref().unwrap(), "restore", None);
    history.history_request_id = Some(keep.request_id);
    history.direction = Some("undo".into());
    let restored = resolve(&f, history);
    assert_eq!(restored.review.as_ref().unwrap().state, "pending");
    assert_eq!(restored.review.as_ref().unwrap().pages[0].chunks.len(), 2);
    assert!(restored.pages.is_empty());
    assert_eq!(
        note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
            .unwrap()
            .revision,
        revision
    );
}

#[tokio::test]
async fn delivery_failure_rolls_back_bodies_and_review_checkpoints_together() {
    let f = rich_fixture();
    let before = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap();
    read(&f, &path(f.page)).await;
    f.tools.store.connection().unwrap().execute_batch("create trigger fail_live_save before update on note_pages begin select raise(abort,'injected checkpoint failure'); end;").unwrap();
    let output = f
        .tools
        .apply(
            "failed-live-delivery",
            &patch(&path(f.page), "first old", "first new"),
        )
        .await
        .unwrap();
    assert_eq!(output["targets"][0]["status"], "failed");
    let db = f.tools.store.connection().unwrap();
    assert_eq!(
        serde_json::to_value(note::load(&db, &f.page.to_string()).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_note_patch_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
