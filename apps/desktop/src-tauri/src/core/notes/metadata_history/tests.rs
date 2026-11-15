use super::*;
use crate::notes::{
    document_repository,
    model::{NoteDocument, NoteDocumentSaveRequest, NotePageDraft},
    page_creation::create_page_from,
};
use serde_json::json;
fn fixture() -> (Connection, String, String) {
    let mut db = crate::notes::tests::migrated_connection();
    let draft = |title: &str| NotePageDraft {
        title: title.into(),
        parent_page_id: None,
        parent_block_position: None,
    };
    let parent = create_page_from(&mut db, draft("Parent")).unwrap();
    let child = create_page_from(&mut db, draft("Child")).unwrap();
    let mut doc = parent.document.clone();
    doc.blocks=serde_json::from_value(json!([
        {"id":"body","type":"paragraph","props":{},"children":[],"content":{"kind":"inline","items":[{"type":"text","text":"accepted","styles":[]}]}},
        {"id":"entry","type":"researchObjectReference","props":{"pageEntry":true},"children":[],"content":{"kind":"reference","targetKind":"note_page","targetId":child.page.id,"labelSnapshot":"Child","quoteSnapshot":""}},
        {"id":"ordinary","type":"researchObjectReference","props":{},"children":[],"content":{"kind":"reference","targetKind":"note_page","targetId":child.page.id,"labelSnapshot":"Child","quoteSnapshot":""}}
    ])).unwrap();
    document_repository::save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: doc.clone(),
            expected_revision: parent.page.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    crate::notes::hbs::store_baseline(&tx, &parent.page.id, &doc).unwrap();
    tx.commit().unwrap();
    (db, parent.page.id, child.page.id)
}
fn deletion(db: &mut Connection, id: &str) -> BatchReceipt {
    let page = page_from(db, id).unwrap();
    metadata_batch::apply_from(
        db,
        BatchRequest {
            operation_id: "delete".into(),
            action: "trash".into(),
            items: vec![BatchItem {
                id: id.into(),
                expected_revision: page.revision,
            }],
        },
    )
    .unwrap()
}
fn request(db: &Connection, id: &str, direction: &str, operation: &str) -> ReplayRequest {
    serde_json::from_value(json!({"operationId":operation,"sourceOperationId":"delete","direction":direction,"items":[{"id":id,"expectedRevision":page_from(db,id).unwrap().revision}]})).unwrap()
}
#[test]
fn undo_redo_uses_the_same_atomic_metadata_receipts_and_keeps_later_text_and_ordinary_links() {
    let (mut db, parent, child) = fixture();
    deletion(&mut db, &child);
    let mut source = document_repository::load_document_from(&db, &parent).unwrap();
    source.document.blocks[0].content["items"][0]["text"] = json!("later pending text");
    document_repository::save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: source.document,
            expected_revision: source.page.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    for (direction, id) in [("undo", "u1"), ("redo", "r1"), ("undo", "u2")] {
        let req = request(&db, &child, direction, id);
        let encoded = serde_json::to_value(&req).unwrap();
        let receipt = replay_from(&mut db, req).unwrap();
        let duplicate = replay_from(&mut db, serde_json::from_value(encoded).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(receipt).unwrap(),
            serde_json::to_value(duplicate).unwrap()
        );
        let doc = document_repository::load_document_value(&db, &parent).unwrap();
        assert_eq!(
            doc.blocks[0].content["items"][0]["text"],
            "later pending text"
        );
        assert!(doc.blocks.iter().any(|v| v.id == "ordinary"));
        assert_eq!(
            doc.blocks.iter().any(|v| v.id == "entry"),
            direction == "undo"
        );
        assert_eq!(
            page_from(&db, &child).unwrap().status,
            if direction == "undo" {
                "active"
            } else {
                "trashed"
            }
        );
        let raw: String = db
            .query_row(
                "select baseline_json from hbs_note_baselines where page_id=?1",
                [&parent],
                |r| r.get(0),
            )
            .unwrap();
        let baseline: NoteDocument = serde_json::from_str(&raw).unwrap();
        assert_eq!(baseline.blocks[0].content["items"][0]["text"], "accepted");
        assert_eq!(
            baseline.blocks.iter().any(|v| v.id == "entry"),
            direction == "undo"
        );
    }
}
#[test]
fn failed_entry_restore_rolls_back_status_body_and_replay_receipt() {
    let (mut db, parent, child) = fixture();
    deletion(&mut db, &child);
    db.execute(
        "update hbs_note_baselines set baseline_json='{}' where page_id=?1",
        [&parent],
    )
    .unwrap();
    let req = request(&db, &child, "undo", "failed");
    assert!(replay_from(&mut db, req).is_err());
    assert_eq!(page_from(&db, &child).unwrap().status, "trashed");
    assert!(
        !document_repository::load_document_value(&db, &parent)
            .unwrap()
            .blocks
            .iter()
            .any(|v| v.id == "entry")
    );
    let count: i64 = db
        .query_row(
            "select count(*) from note_metadata_batch_receipts where operation_id='failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn undo_restores_archived_status_and_rejects_stale_or_retargeted_requests() {
    let (mut db, _parent, child) = fixture();
    let current = page_from(&db, &child).unwrap();
    metadata_batch::apply_from(
        &mut db,
        BatchRequest {
            operation_id: "archive".into(),
            action: "archive".into(),
            items: vec![BatchItem {
                id: child.clone(),
                expected_revision: current.revision,
            }],
        },
    )
    .unwrap();
    deletion(&mut db, &child);
    let req = request(&db, &child, "undo", "u");
    replay_from(&mut db, req).unwrap();
    assert_eq!(page_from(&db, &child).unwrap().status, "archived");
    let req = request(&db, &child, "undo", "stale");
    assert!(replay_from(&mut db, req).is_err());
}
#[test]
fn undo_does_not_restore_a_previously_trashed_legacy_descendant() {
    let (mut db, parent, _child) = fixture();
    let old = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Old".into(),
            parent_page_id: Some(parent.clone()),
            parent_block_position: None,
        },
    )
    .unwrap();
    metadata_batch::apply_from(
        &mut db,
        BatchRequest {
            operation_id: "old-delete".into(),
            action: "trash".into(),
            items: vec![BatchItem {
                id: old.page.id.clone(),
                expected_revision: old.page.revision,
            }],
        },
    )
    .unwrap();
    deletion(&mut db, &parent);
    let req = request(&db, &parent, "undo", "parent-undo");
    replay_from(&mut db, req).unwrap();
    assert_eq!(page_from(&db, &parent).unwrap().status, "active");
    assert_eq!(page_from(&db, &old.page.id).unwrap().status, "trashed");
}
