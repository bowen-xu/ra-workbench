use super::*;
use crate::notes::{
    hbs,
    metadata_batch::{BatchItem, BatchRequest, apply_from},
    model::{NoteDocumentSnapshot, NotePageDraft},
    page_creation::create_page_from,
};
use rusqlite::Connection;
use serde_json::{Value, json};

fn page(db: &mut Connection, title: &str, parent: Option<String>) -> NoteDocumentSnapshot {
    create_page_from(
        db,
        NotePageDraft {
            title: title.into(),
            parent_page_id: parent,
            parent_block_position: None,
        },
    )
    .unwrap()
}
fn link(id: &str, target: &str, entry: bool) -> NoteDocumentBlock {
    serde_json::from_value(json!({"id":id,"type":"researchObjectReference","props":if entry {json!({"pageEntry":true})} else {json!({})},"children":[],"content":{"kind":"reference","targetKind":"note_page","targetId":target,"labelSnapshot":"Child","quoteSnapshot":""}})).unwrap()
}
fn commit(
    db: &mut Connection,
    page: &NoteDocumentSnapshot,
    blocks: Vec<NoteDocumentBlock>,
    baseline: Option<NoteDocument>,
) -> Value {
    let mut document = page.document.clone();
    document.blocks = blocks;
    let saved = serde_json::to_value(&document).unwrap();
    let tx = db.transaction().unwrap();
    let result = hbs::commit(
        &tx,
        &page.page.id,
        page.page.revision,
        saved.clone(),
        baseline
            .map(|v| serde_json::to_value(v).unwrap())
            .unwrap_or(saved),
    )
    .unwrap();
    tx.commit().unwrap();
    result
}
fn trash(db: &mut Connection, id: &str, operation: &str) -> Value {
    let revision = crate::notes::page_repository::page_from(db, id)
        .unwrap()
        .revision;
    serde_json::to_value(
        apply_from(
            db,
            BatchRequest {
                operation_id: operation.into(),
                action: "trash".into(),
                items: vec![BatchItem {
                    id: id.into(),
                    expected_revision: revision,
                }],
            },
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn removes_only_marked_entrances_and_preserves_pending_body_and_nested_text() {
    let mut db = crate::notes::tests::migrated_connection();
    let parent = page(&mut db, "Parent", None);
    let child = page(&mut db, "Child", None);
    let paragraph = |text: &str| {
        serde_json::from_value::<NoteDocumentBlock>(json!({"id":"body","type":"paragraph","props":{},"children":[],"content":{"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}})).unwrap()
    };
    let mut entry = link("entry", &child.page.id, true);
    let mut nested = paragraph("preserve nested");
    nested.id = "nested".into();
    entry.children.push(nested);
    commit(
        &mut db,
        &parent,
        vec![
            paragraph("accepted"),
            entry.clone(),
            link("ordinary", &child.page.id, false),
        ],
        None,
    );
    let before = document_repository::load_document_from(&db, &parent.page.id).unwrap();
    commit(
        &mut db,
        &before,
        vec![
            paragraph("pending"),
            entry,
            link("ordinary", &child.page.id, false),
        ],
        Some(before.document.clone()),
    );
    let receipt = trash(&mut db, &child.page.id, "trash-entry");
    let after = hbs::read(&db, &parent.page.id).unwrap();
    assert_eq!(
        after["saved"]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["body", "nested", "ordinary"]
    );
    assert_eq!(
        after["saved"]["blocks"][0]["content"]["items"][0]["text"],
        "pending"
    );
    assert_eq!(
        after["baseline"]["blocks"][0]["content"]["items"][0]["text"],
        "accepted"
    );
    assert_eq!(after["baseline"]["blocks"][1]["id"], "nested");
    assert_eq!(receipt["bodyChangedIds"], json!([parent.page.id]));
}
#[test]
fn recognizes_older_creation_receipt_and_keeps_a_later_ordinary_reference() {
    let mut db = crate::notes::tests::migrated_connection();
    let parent = page(&mut db, "Parent", None);
    let mut source = parent.document.clone();
    source.blocks = vec![link("older-entry", "child-created", false)];
    let document = json!({"schemaVersion":1,"pageId":"child-created","title":"Child","blocks":[]});
    let request=serde_json::from_value(json!({"commit":{"id":"create","expectedVersion":0,"saved":document,"baseline":document},"favorite":false,"source":{"document":source,"expectedRevision":parent.page.revision,"saveReason":"structure"}})).unwrap();
    crate::notes::draft::publish(&mut db, "child-created", request).unwrap();
    let mut source = document_repository::load_document_from(&db, &parent.page.id).unwrap();
    source
        .document
        .blocks
        .push(link("ordinary", "child-created", false));
    commit(&mut db, &source, source.document.blocks.clone(), None);
    trash(&mut db, "child-created", "trash-old-entry");
    let after = hbs::read(&db, &parent.page.id).unwrap();
    assert_eq!(after["saved"], after["baseline"]);
    assert_eq!(after["saved"]["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(after["saved"]["blocks"][0]["id"], "ordinary");
}
#[test]
fn legacy_entry_lifecycle_advances_only_its_baseline_scope_and_restores_the_entrance() {
    let mut db = crate::notes::tests::migrated_connection();
    let parent = page(&mut db, "Parent", None);
    let child = page(&mut db, "Child", Some(parent.page.id.clone()));
    let before = document_repository::load_document_from(&db, &parent.page.id).unwrap();
    let mut blocks = before.document.blocks.clone();
    blocks.push(link("ordinary", &child.page.id, false));
    commit(&mut db, &before, blocks, None);
    trash(&mut db, &child.page.id, "legacy-trash");
    let removed = hbs::read(&db, &parent.page.id).unwrap();
    assert_eq!(removed["saved"], removed["baseline"]);
    let revision = crate::notes::page_repository::page_from(&db, &child.page.id)
        .unwrap()
        .revision;
    apply_from(
        &mut db,
        BatchRequest {
            operation_id: "restore".into(),
            action: "restore".into(),
            items: vec![BatchItem {
                id: child.page.id,
                expected_revision: revision,
            }],
        },
    )
    .unwrap();
    let restored = hbs::read(&db, &parent.page.id).unwrap();
    assert_eq!(restored["saved"], restored["baseline"]);
    assert!(
        restored["saved"]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["type"] == "childPage")
    );
}

#[test]
fn failure_to_preserve_baseline_rolls_back_trash_and_entry_removal() {
    let mut db = crate::notes::tests::migrated_connection();
    let parent = page(&mut db, "Parent", None);
    let child = page(&mut db, "Child", None);
    commit(
        &mut db,
        &parent,
        vec![link("entry", &child.page.id, true)],
        None,
    );
    db.execute(
        "update hbs_note_baselines set baseline_json='{}' where page_id=?1",
        [&parent.page.id],
    )
    .unwrap();
    let result = apply_from(
        &mut db,
        BatchRequest {
            operation_id: "failed-trash".into(),
            action: "trash".into(),
            items: vec![BatchItem {
                id: child.page.id.clone(),
                expected_revision: child.page.revision,
            }],
        },
    );
    assert!(result.is_err());
    assert_eq!(
        crate::notes::page_repository::page_from(&db, &child.page.id)
            .unwrap()
            .status,
        "active"
    );
    assert_eq!(
        document_repository::load_document_value(&db, &parent.page.id)
            .unwrap()
            .blocks[0]
            .id,
        "entry"
    );
    assert_eq!(
        db.query_row(
            "select count(*) from note_metadata_batch_receipts where operation_id='failed-trash'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
