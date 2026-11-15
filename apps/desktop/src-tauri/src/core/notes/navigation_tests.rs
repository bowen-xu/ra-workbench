use super::{
    document_repository::{load_document_from, save_document_from},
    favorites::{ReorderFavorites, list_from, reorder_from},
    model::{NoteDocumentBlock, NoteDocumentSaveRequest, NotePageDraft, NotePageMetadataUpdate},
    navigation::{NavigationCommit, commit_from},
    page_repository::{create_page_from, update_page_from},
};
use rusqlite::Connection;
fn page(db: &mut Connection, title: &str) -> super::model::NoteDocumentSnapshot {
    create_page_from(
        db,
        NotePageDraft {
            title: title.into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap()
}
fn favorite(db: &mut Connection, id: &str) {
    let before = load_document_from(db, id).unwrap();
    update_page_from(
        db,
        NotePageMetadataUpdate {
            page_id: id.into(),
            expected_revision: before.page.revision,
            title: None,
            is_favorite: Some(true),
            project_ids: None,
            tag_ids: None,
        },
    )
    .unwrap();
}
#[test]
fn favorite_order_upgrades_188_reopens_and_rejects_stale_reorders_without_touching_edit_times() {
    let path = std::env::temp_dir().join(format!("favorite-order-{}.sqlite", uuid::Uuid::new_v4()));
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 188);
    let a = page(&mut db, "a");
    let b = page(&mut db, "b");
    favorite(&mut db, &a.page.id);
    favorite(&mut db, &b.page.id);
    db.execute(
        "update note_pages set updated_at='2026-10-20T00:00:00Z' where id=?1",
        [&a.page.id],
    )
    .unwrap();
    db.execute(
        "update note_pages set updated_at='2026-10-21T00:00:00Z' where id=?1",
        [&b.page.id],
    )
    .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        list_from(&db).unwrap(),
        vec![b.page.id.clone(), a.page.id.clone()]
    );
    let c = page(&mut db, "c");
    favorite(&mut db, &c.page.id);
    let before = list_from(&db).unwrap();
    assert_eq!(before[0], c.page.id);
    let desired = vec![a.page.id.clone(), c.page.id.clone(), b.page.id.clone()];
    reorder_from(
        &mut db,
        ReorderFavorites {
            expected_page_ids: before.clone(),
            page_ids: desired.clone(),
        },
    )
    .unwrap();
    assert!(
        reorder_from(
            &mut db,
            ReorderFavorites {
                expected_page_ids: before,
                page_ids: desired.clone()
            }
        )
        .is_err()
    );
    assert_eq!(
        load_document_from(&db, &a.page.id).unwrap().page.updated_at,
        "2026-10-20T00:00:00Z"
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(list_from(&db).unwrap(), desired);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn navigation_rolls_back_all_bodies_and_favorites_when_a_later_cas_fails() {
    let mut db = super::tests::migrated_connection();
    let mut a = page(&mut db, "a");
    let b = page(&mut db, "b");
    let target = page(&mut db, "target");
    let link = NoteDocumentBlock {
        id: "stable-link".into(),
        block_type: "researchObjectReference".into(),
        props: serde_json::json!({}),
        children: vec![],
        content: serde_json::json!({"kind":"reference","targetKind":"note_page","targetId":target.page.id,"labelSnapshot":"target","quoteSnapshot":""}),
    };
    a.document.blocks = vec![link.clone()];
    a = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: a.document,
            expected_revision: 0,
            save_reason: "structure".into(),
        },
    )
    .unwrap();
    let request = |expected| {
        let mut source = a.document.clone();
        source.blocks.clear();
        let mut destination = b.document.clone();
        destination.blocks = vec![link.clone()];
        NavigationCommit {
            documents: vec![
                NoteDocumentSaveRequest {
                    document: source,
                    expected_revision: a.page.revision,
                    save_reason: "structure".into(),
                },
                NoteDocumentSaveRequest {
                    document: destination,
                    expected_revision: expected,
                    save_reason: "structure".into(),
                },
            ],
            expected_favorite_ids: vec![],
            favorite_ids: vec![target.page.id.clone()],
        }
    };
    assert!(commit_from(&mut db, request(99)).is_err());
    assert_eq!(
        load_document_from(&db, &a.page.id).unwrap().document.blocks[0].id,
        "stable-link"
    );
    assert!(list_from(&db).unwrap().is_empty());
    let result = commit_from(&mut db, request(0)).unwrap();
    assert!(
        result
            .iter()
            .find(|value| value.page.id == a.page.id)
            .unwrap()
            .document
            .blocks
            .is_empty()
    );
    assert_eq!(
        load_document_from(&db, &b.page.id).unwrap().document.blocks[0].id,
        "stable-link"
    );
    assert_eq!(list_from(&db).unwrap(), vec![target.page.id]);
}
