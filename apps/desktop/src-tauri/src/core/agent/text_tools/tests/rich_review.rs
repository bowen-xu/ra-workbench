use super::*;
use crate::agent::text_tools::note_review::{self, Receipt, Request};
use crate::notes::{NoteDocument, NoteDocumentBlock};
#[path = "rich_review/agent_history.rs"]
mod agent_history;
#[path = "rich_review/metadata.rs"]
mod metadata;
#[path = "rich_review/native.rs"]
mod native;
#[path = "rich_review/persistence.rs"]
mod persistence;

fn block(id: &str, text: &str) -> NoteDocumentBlock {
    NoteDocumentBlock {
        id: id.into(),
        block_type: "paragraph".into(),
        props: json!({"privateSidecar":"kept"}),
        content: json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}),
        children: vec![],
    }
}
fn rich_fixture() -> Fixture {
    let fixture = fixture(true);
    let db = fixture.tools.store.connection().unwrap();
    let current = note::load(&db, &fixture.page.to_string()).unwrap();
    let document = NoteDocument {
        title: None,
        schema_version: 1,
        page_id: current.page_id.clone(),
        blocks: vec![
            block("first", "first old"),
            block("middle", "middle unchanged"),
            block("third", "third old"),
        ],
    };
    let tx = db.unchecked_transaction().unwrap();
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: current.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    fixture
}
async fn two_changes(f: &Fixture) -> decisions::Review {
    read(f, &path(f.page)).await;
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n-first old\n+first new\n@@\n-third old\n+third new\n*** End Patch",
        path(f.page)
    );
    let result = f.tools.apply("two-changes", &patch).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    review(f)
}
fn request(f: &Fixture, review: &decisions::Review, action: &str, chunk: Option<usize>) -> Request {
    Request {
        conversation_id: f.tools.record.conversation_id,
        proposal_id: review.proposal_id.clone(),
        generation: review.generation,
        request_id: Uuid::new_v4(),
        page_id: f.page.to_string(),
        action: action.into(),
        chunk_id: chunk.map(|i| {
            review
                .pages
                .iter()
                .find(|p| p.page_id == f.page.to_string())
                .unwrap()
                .chunks[i]
                .id
                .clone()
        }),
        document: None,
        history_request_id: None,
        direction: None,
        expected_revision: None,
    }
}
fn body(document: &NoteDocument, index: usize) -> &str {
    document.blocks[index].content["items"][0]["text"]
        .as_str()
        .unwrap()
}
fn resolve(f: &Fixture, request: Request) -> Receipt {
    note_review::resolve_in(&f.tools.store, request).unwrap()
}

#[tokio::test]
async fn partial_keep_and_undo_move_only_the_review_baseline_and_commit_resolved_body() {
    let f = rich_fixture();
    let initial = two_changes(&f).await;
    assert_eq!(initial.pages[0].chunks.len(), 2);
    let keep = request(&f, &initial, "keep", Some(0));
    let receipt = resolve(&f, keep.clone());
    assert!(receipt.pages.is_empty());
    assert_eq!(
        serde_json::to_value(resolve(&f, keep.clone())).unwrap(),
        serde_json::to_value(&receipt).unwrap()
    );
    assert!(
        note_review::resolve_in(
            &f.tools.store,
            Request {
                action: "undo".into(),
                ..keep
            }
        )
        .err()
        .unwrap()
        .contains("identity")
    );
    let shown = receipt.review.unwrap();
    assert_eq!(shown.pages[0].chunks.len(), 1);
    assert_eq!(
        body(shown.pages[0].review_document.as_ref().unwrap(), 0),
        "first new"
    );
    assert_eq!(
        body(shown.pages[0].before_document.as_ref().unwrap(), 0),
        "first old"
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
    let done = resolve(&f, request(&f, &shown, "undo", Some(0)));
    assert_eq!(done.review.as_ref().unwrap().state, "accept");
    assert_eq!(body(&done.pages[0].document, 0), "first new");
    assert_eq!(body(&done.pages[0].document, 2), "third old");
    assert_eq!(
        done.pages[0].document.blocks[0].props["privateSidecar"],
        "kept"
    );
    let undone = decisions::decide(
        &f.tools.store,
        decision(&f, done.review.as_ref().unwrap(), "undo"),
    )
    .unwrap();
    assert_eq!(body(&undone.pages[0].document, 0), "first old");
}

#[tokio::test]
async fn kept_baseline_survives_later_model_patch_without_reappearing_as_original_diff() {
    let f = rich_fixture();
    let first = two_changes(&f).await;
    let kept = resolve(&f, request(&f, &first, "keep", Some(0)))
        .review
        .unwrap();
    read(&f, &path(f.page)).await;
    f.tools
        .apply("later", &patch(&path(f.page), "first new", "first newer"))
        .await
        .unwrap();
    let latest = review(&f);
    assert_ne!(latest.proposal_id, kept.proposal_id);
    assert_eq!(latest.pages[0].chunks.len(), 2);
    assert_eq!(
        body(latest.pages[0].review_document.as_ref().unwrap(), 0),
        "first new"
    );
    let frozen = note_review::snapshots::read(
        &f.tools.store.connection().unwrap(),
        f.tools.record.conversation_id,
        &latest.proposal_id,
        latest.generation,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        body(frozen.pages[0].before_document.as_ref().unwrap(), 0),
        "first new"
    );
    assert_eq!(
        body(frozen.pages[0].before_document.as_ref().unwrap(), 2),
        "third new"
    );
    assert_eq!(frozen.pages[0].chunks.len(), 1);
    assert!(note_review::resolve_in(&f.tools.store, request(&f, &kept, "undo", Some(0))).is_err());
    let kept_all = resolve(&f, request(&f, &latest, "keep", None));
    assert_eq!(body(&kept_all.pages[0].document, 0), "first newer");
}

#[tokio::test]
async fn normal_candidate_edits_sync_unreviewed_roots_and_preserve_pending_roots_and_sidecars() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let mut input = request(&f, &shown, "edit", None);
    let mut edited = shown.pages[0].after_document.as_ref().unwrap().clone();
    edited.blocks[0].content["items"][0]["text"] = json!("first user revised");
    edited.blocks[1].content["items"][0]["text"] = json!("middle manual");
    for block in &mut edited.blocks {
        block.props =
            json!({"textAlignment":"left","textColor":"default","backgroundColor":"default"});
    }
    input.document = Some(edited);
    let result = resolve(&f, input).review.unwrap();
    assert_eq!(
        body(result.pages[0].review_document.as_ref().unwrap(), 0),
        "first old"
    );
    assert_eq!(
        body(result.pages[0].review_document.as_ref().unwrap(), 1),
        "middle manual"
    );
    assert_eq!(
        result.pages[0].after_document.as_ref().unwrap().blocks[1].props["privateSidecar"],
        "kept"
    );
    assert!(
        result.pages[0].after_document.as_ref().unwrap().blocks[1]
            .props
            .get("textAlignment")
            .is_none()
    );
    assert_eq!(result.pages[0].chunks.len(), 2);
    let saved = resolve(&f, request(&f, &result, "keep", None));
    assert_eq!(body(&saved.pages[0].document, 1), "middle manual");
    assert_eq!(body(&saved.pages[0].document, 0), "first user revised");
}

#[tokio::test]
async fn keep_all_history_restores_review_without_rewinding_or_resaving_formal_document() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let keep = request(&f, &shown, "keep", None);
    let accepted = resolve(&f, keep.clone());
    let formal = accepted.pages[0].clone();
    let mut undo = request(&f, accepted.review.as_ref().unwrap(), "restore", None);
    undo.history_request_id = Some(keep.request_id);
    undo.direction = Some("undo".into());
    let reopened = resolve(&f, undo);
    assert!(reopened.pages.is_empty());
    let reopened_review = reopened.review.unwrap();
    assert_eq!(reopened_review.state, "pending");
    assert_eq!(reopened_review.pages[0].chunks.len(), 2);
    assert_eq!(
        body(
            reopened_review.pages[0].review_document.as_ref().unwrap(),
            0
        ),
        "first old"
    );
    assert_eq!(
        serde_json::to_value(
            note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap()
        )
        .unwrap(),
        serde_json::to_value(&formal).unwrap()
    );
    let mut redo = request(&f, &reopened_review, "restore", None);
    redo.history_request_id = Some(keep.request_id);
    redo.direction = Some("redo".into());
    let redone = resolve(&f, redo);
    assert_eq!(redone.review.as_ref().unwrap().state, "accept");
    assert_eq!(redone.pages[0].revision, formal.revision);
    assert_eq!(
        serde_json::to_value(&redone.pages[0]).unwrap(),
        serde_json::to_value(formal).unwrap()
    );
    let mut again = request(&f, redone.review.as_ref().unwrap(), "restore", None);
    again.history_request_id = Some(keep.request_id);
    again.direction = Some("undo".into());
    let second_undo = resolve(&f, again);
    assert_eq!(second_undo.review.unwrap().pages[0].chunks.len(), 2);
    assert!(second_undo.pages.is_empty());
}

#[tokio::test]
async fn stale_formal_version_and_stale_chunk_fail_without_erasing_input_or_baseline() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let mut bad = request(&f, &shown, "keep", Some(0));
    bad.chunk_id = Some("missing".into());
    assert!(note_review::resolve_in(&f.tools.store, bad).is_err());
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set revision=revision+1 where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    assert!(
        note_review::resolve_in(&f.tools.store, request(&f, &shown, "keep", None))
            .err()
            .unwrap()
            .contains("saved note changed")
    );
    let still_pending = review(&f);
    assert_eq!(still_pending.generation, shown.generation);
    assert_eq!(still_pending.pages[0].chunks.len(), 2);
}

#[tokio::test]
async fn undo_all_saves_the_restored_body_without_waiting_for_another_keep() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let revision = note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
        .unwrap()
        .revision;
    let undone = resolve(&f, request(&f, &shown, "undo", None));
    assert_eq!(undone.review.unwrap().state, "reject");
    assert_eq!(body(&undone.pages[0].document, 0), "first old");
    assert_eq!(
        note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
            .unwrap()
            .revision,
        revision + 1
    );
}

#[tokio::test]
async fn reopen_restores_partial_baseline_and_original_receipt_without_reexecution() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let keep = request(&f, &shown, "keep", Some(0));
    let kept = resolve(&f, keep.clone());
    let mut connection = Connection::open(f._root.path().join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    let reopened = AppStore::for_test(connection, f._root.path().join("objects"));
    let replayed = note_review::resolve_in(&reopened, keep).unwrap();
    assert_eq!(
        serde_json::to_value(replayed).unwrap(),
        serde_json::to_value(&kept).unwrap()
    );
    let recovered = decisions::review_selected(
        &reopened.connection().unwrap(),
        f.tools.record.conversation_id,
        Some(&shown.proposal_id),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(recovered.generation, kept.generation);
    assert_eq!(
        body(recovered.pages[0].review_document.as_ref().unwrap(), 0),
        "first new"
    );
    assert_eq!(recovered.pages[0].chunks.len(), 1);
}

#[tokio::test]
async fn final_keep_failure_rolls_back_candidate_baseline_history_and_formal_receipt_together() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let request = request(&f, &shown, "keep", None);
    let request_id = request.request_id;
    f.tools.store.connection().unwrap().execute_batch("create trigger fail_rich_keep before update on agent_note_patch_candidates begin select raise(abort,'injected'); end;").unwrap();
    assert!(note_review::resolve_in(&f.tools.store, request).is_err());
    let still = review(&f);
    assert_eq!(still.generation, shown.generation);
    assert_eq!(still.pages[0].chunks.len(), 2);
    let db = f.tools.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_note_review_receipts where request_id=?1",
            [request_id.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_note_patch_decisions where request_id=?1",
            [request_id.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        body(&note::load(&db, &f.page.to_string()).unwrap().document, 0),
        "first new"
    );
}

#[tokio::test]
async fn revoked_write_stops_partial_keep_and_candidate_typing_without_advancing_generation() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let mut input = f.tools.record.input.clone();
    for source in &mut input.context {
        let mut anchor: Value = serde_json::from_str(source.anchor.as_ref().unwrap()).unwrap();
        anchor["contextAccess"] = json!("readOnly");
        source.anchor = Some(anchor.to_string());
    }
    {
        let mut db = f.tools.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        submissions::insert_in(&tx, f.tools.record.conversation_id, Uuid::new_v4(), input).unwrap();
        tx.commit().unwrap();
    }
    assert!(note_review::resolve_in(&f.tools.store, request(&f, &shown, "keep", Some(0))).is_err());
    let mut edit = request(&f, &shown, "edit", None);
    edit.document = shown.pages[0].after_document.clone();
    assert!(note_review::resolve_in(&f.tools.store, edit).is_err());
    assert_eq!(review(&f).generation, shown.generation);
}

#[tokio::test]
async fn frozen_tool_snapshot_keeps_original_statistics_and_uses_live_decision_generation() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let frozen = note_review::snapshots::read(
        &f.tools.store.connection().unwrap(),
        f.tools.record.conversation_id,
        &shown.proposal_id,
        shown.generation,
    )
    .unwrap()
    .unwrap();
    let kept = resolve(&f, request(&f, &shown, "keep", Some(0)))
        .review
        .unwrap();
    let accepted = resolve(&f, request(&f, &kept, "undo", Some(0)))
        .review
        .unwrap();
    let later = note_review::snapshots::read(
        &f.tools.store.connection().unwrap(),
        f.tools.record.conversation_id,
        &shown.proposal_id,
        shown.generation,
    )
    .unwrap()
    .unwrap();
    assert_eq!(later.pages[0].before, frozen.pages[0].before);
    assert_eq!(later.pages[0].after, frozen.pages[0].after);
    assert_eq!(later.generation, shown.generation);
    assert_eq!(later.operation_generation, accepted.generation);
    assert_eq!(later.state, "accept");
    assert!(later.whole_undo_available);
    assert!(
        note_review::snapshots::read(
            &f.tools.store.connection().unwrap(),
            f.tools.record.conversation_id,
            &shown.proposal_id,
            999
        )
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn whole_group_undo_and_reject_cannot_erase_manual_candidate_input() {
    let f = rich_fixture();
    let shown = two_changes(&f).await;
    let mut edit = request(&f, &shown, "edit", None);
    let mut document = shown.pages[0].after_document.as_ref().unwrap().clone();
    document.blocks[1].content["items"][0]["text"] = json!("manual preserved");
    edit.document = Some(document);
    let edited = resolve(&f, edit).review.unwrap();
    assert!(!edited.whole_undo_available);
    assert!(decisions::decide(&f.tools.store, decision(&f, &edited, "reject")).is_err());
    let accepted = resolve(&f, request(&f, &edited, "keep", None))
        .review
        .unwrap();
    assert!(!accepted.whole_undo_available);
    assert!(decisions::decide(&f.tools.store, decision(&f, &accepted, "undo")).is_err());
    assert_eq!(
        body(
            &note::load(&f.tools.store.connection().unwrap(), &f.page.to_string())
                .unwrap()
                .document,
            1
        ),
        "manual preserved"
    );
}
