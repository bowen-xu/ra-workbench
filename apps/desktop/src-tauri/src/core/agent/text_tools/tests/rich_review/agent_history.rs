use super::*;

#[tokio::test]
async fn agent_installation_redo_reverses_the_first_undo_receipt_into_pending_review() {
    let f = rich_fixture();
    let initial = two_changes(&f).await;
    let mut undo = request(&f, &initial, "edit", None);
    undo.document = initial.pages[0].before_document.clone();
    let undone = resolve(&f, undo.clone());
    assert!(undone.review.as_ref().unwrap().pages[0].chunks.is_empty());
    let mut current = undone.review.unwrap();
    for _ in 0..2 {
        let mut redo = request(&f, &current, "restore", None);
        redo.history_request_id = Some(undo.request_id);
        redo.direction = Some("undo".into());
        let restored = resolve(&f, redo);
        assert_eq!(body(&restored.pages[0].document, 0), "first new");
        current = restored.review.unwrap();
        assert_eq!(current.state, "pending");
        assert_eq!(current.pages[0].chunks.len(), 2);
        assert_eq!(
            body(current.pages[0].review_document.as_ref().unwrap(), 0),
            "first old"
        );
        assert_eq!(
            body(current.pages[0].after_document.as_ref().unwrap(), 0),
            "first new"
        );
        assert_eq!(
            body(
                &note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
                    .unwrap()
                    .document,
                0
            ),
            "first new"
        );
        let mut again = request(&f, &current, "restore", None);
        again.history_request_id = Some(undo.request_id);
        again.direction = Some("redo".into());
        current = resolve(&f, again).review.unwrap();
        assert!(current.pages[0].chunks.is_empty());
    }
}
