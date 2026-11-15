use super::*;
use crate::notes::{
    model::{NoteDocumentBlock, NotePageDraft},
    page_repository::create_page_from,
    tests::{insert_paper_fixture, migrated_connection},
};
use serde_json::json;

fn fixture() -> (Connection, NoteDocumentSaveRequest) {
    let mut db = migrated_connection();
    insert_paper_fixture(&db);
    let parent = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Atomic note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap();
    create_page_from(
        &mut db,
        NotePageDraft {
            title: "Child evidence".into(),
            parent_page_id: Some(parent.page.id.clone()),
            parent_block_position: None,
        },
    )
    .unwrap();
    let mut before = load_document_from(&db, &parent.page.id).unwrap();
    before.document.blocks.extend([
        NoteDocumentBlock {
            id: "paragraph".into(), block_type: "paragraph".into(), props: json!({}),
            content: json!({"kind":"inline","items":[{"type":"text","text":"Original","styles":[]},
                {"type":"mention","targetKind":"paper","targetId":"paper-1","labelSnapshot":"Evidence"}]}),
            children: vec![],
        },
        NoteDocumentBlock {
            id: "asset".into(), block_type: "image".into(), props: json!({}),
            content: json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"Evidence","mimeType":"application/pdf","caption":"Source"}),
            children: vec![],
        },
    ]);
    let saved = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: before.document,
            expected_revision: before.page.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    db.execute("update note_index_state set state='ready' where id=1", [])
        .unwrap();
    let mut changed = saved.document;
    changed.blocks.retain(|block| block.id == "paragraph");
    changed.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":"Revised","styles":[]}]});
    (
        db,
        NoteDocumentSaveRequest {
            document: changed,
            expected_revision: saved.page.revision,
            save_reason: "manual".into(),
        },
    )
}

// Include dependencies that could otherwise survive a partially applied save:
// child lifecycle, backlinks, current assets, retained history and index state.
fn state(db: &Connection) -> Vec<Vec<Vec<String>>> {
    [
        "select id,parent_page_id,title,position,revision,status,trashed_at,updated_at from note_pages order by id",
        "select id,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,status,trashed_at,updated_at from note_blocks order by id",
        "select id,page_id,page_revision,document_json,reason from note_revisions order by id",
        "select id,source_page_id,source_block_id,target_kind,target_id,target_snapshot_json from note_links order by id",
        "select block_id,content_sha256 from note_block_assets order by block_id,content_sha256",
        "select revision_id,content_sha256 from note_revision_assets order by revision_id,content_sha256",
        "select id,state from note_index_state order by id",
    ].into_iter().map(|sql| {
        let mut query = db.prepare(sql).unwrap();
        let columns = query.column_count();
        query.query_map([], |row| Ok((0..columns).map(|i| format!("{:?}", row.get_ref(i).unwrap())).collect()))
            .unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    }).collect()
}

#[test]
fn outer_abort_rolls_back_body_children_links_assets_history_and_index() {
    let (mut db, request) = fixture();
    let original = state(&db);
    {
        let tx = db.transaction().unwrap();
        let saved = save_document_in(&tx, request).unwrap();
        assert_eq!(saved.document.blocks.len(), 1);
        assert_ne!(state(&tx), original);
        let trashed: i64 = tx
            .query_row(
                "select count(*) from note_pages where status='trashed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(trashed, 1);
        // A subsequent acceptance-ledger failure must be able to undo this save.
        assert!(
            tx.execute("insert into note_pages(id) values(null)", [])
                .is_err()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(state(&db), original);
}

#[test]
fn late_save_failure_rolls_back_every_dependency() {
    let (mut db, request) = fixture();
    let original = state(&db);
    db.execute_batch("create temp trigger fail_note_index before update on note_index_state begin select raise(abort, 'synthetic late failure'); end;").unwrap();
    assert!(save_document_from(&mut db, request).is_err());
    assert_eq!(state(&db), original);
    assert!(db.is_autocommit());
}

#[test]
fn saved_snapshot_and_history_belong_to_the_same_transaction() {
    let (mut db, request) = fixture();
    let before = load_document_from(&db, &request.document.page_id).unwrap();
    let tx = db.transaction().unwrap();
    let saved = save_document_in(&tx, request).unwrap();
    assert_eq!(saved.page.revision, before.page.revision + 1);
    let history: String = tx
        .query_row(
            "select document_json from note_revisions where page_id=?1 and page_revision=?2",
            params![before.page.id, before.page.revision],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&history).unwrap(),
        serde_json::to_value(before.document).unwrap()
    );
    let assets: i64 = tx.query_row("select count(*) from note_revision_assets a join note_revisions r on r.id=a.revision_id where r.page_id=?1 and r.page_revision=?2",
        params![before.page.id,before.page.revision], |r|r.get(0)).unwrap();
    assert_eq!(assets, 1);
    tx.commit().unwrap();
    let loaded = load_document_from(&db, &saved.page.id).unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(loaded).unwrap()
    );
}

#[test]
fn stale_revision_refuses_composed_save_without_changing_the_note() {
    let (mut db, mut request) = fixture();
    request.expected_revision -= 1;
    let original = state(&db);
    let tx = db.transaction().unwrap();
    assert!(matches!(
        save_document_in(&tx, request),
        Err(NotesError::RevisionConflict { .. })
    ));
    tx.rollback().unwrap();
    assert_eq!(state(&db), original);
}

fn repeated_link_fixture(reference: bool) -> (Connection, NoteDocumentSnapshot) {
    let mut db = migrated_connection();
    insert_paper_fixture(&db);
    let page = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Repeated references".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap();
    let mut document = page.document;
    document.blocks = ["remove-first", "remove-second", "keep-reference"].into_iter().map(|id| NoteDocumentBlock {
        id: id.into(), block_type: if reference { "researchObjectReference" } else { "paragraph" }.into(), props: json!({}),
        content: if reference { json!({"kind":"reference","targetKind":"paper","targetId":"paper-1","labelSnapshot":"Paper","quoteSnapshot":"Evidence"}) }
            else { json!({"kind":"inline","items":[{"type":"mention","targetKind":"paper","targetId":"paper-1","labelSnapshot":"Paper"}]}) },
        children: vec![],
    }).collect();
    let saved = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document,
            expected_revision: page.page.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    (db, saved)
}

#[test]
fn hbs_reset_removes_repeated_references_without_identity_collision() {
    let (mut db, saved) = repeated_link_fixture(true);
    crate::notes::hbs::store_baseline(&db, &saved.page.id, &saved.document).unwrap();
    let kept: (String, String) = db
        .query_row(
            "select id,created_at from note_links where source_block_id='keep-reference'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mut initial = saved.document;
    initial.blocks.retain(|block| block.id == "keep-reference");
    let document = serde_json::to_value(&initial).unwrap();
    let receipt = crate::hbs_storage::test_sources::commit(
        &mut db,
        "note",
        &saved.page.id,
        crate::hbs_storage::Commit {
            id: "reset-repeated-links".into(),
            expected_version: saved.page.revision,
            saved: document.clone(),
            baseline: document.clone(),
        },
    )
    .expect("reset must remove obsolete links before their source blocks");
    assert_eq!(receipt["type"], "receipt");
    assert_eq!(receipt["snapshot"]["saved"], document);
    assert_eq!(receipt["snapshot"]["baseline"], document);
    let links: i64 = db
        .query_row(
            "select count(*) from note_links where source_page_id=?1",
            [&saved.page.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(links, 1);
    let after: (String, String) = db
        .query_row(
            "select id,created_at from note_links where source_block_id='keep-reference'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, kept);
}

#[test]
fn removing_mentions_preserves_page_level_links_and_retained_link_metadata() {
    let (mut db, saved) = repeated_link_fixture(false);
    db.execute("insert into note_links(id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at) values('page-link',?1,'paper','paper-1','mention','{\"labelSnapshot\":\"Page relationship\",\"custom\":true}','original-time')", [&saved.page.id]).unwrap();
    db.execute("update note_links set target_snapshot_json=json_set(target_snapshot_json,'$.custom',true) where source_block_id='keep-reference'", []).unwrap();
    let mut document = saved.document;
    document.blocks.retain(|block| block.id == "keep-reference");
    save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document,
            expected_revision: saved.page.revision,
            save_reason: "manual".into(),
        },
    )
    .expect("removed block mentions must not collapse into a page-level relationship");
    let page_link: (String, String) = db
        .query_row(
            "select target_snapshot_json,created_at from note_links where id='page-link'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        page_link,
        (
            "{\"labelSnapshot\":\"Page relationship\",\"custom\":true}".into(),
            "original-time".into()
        )
    );
    let retained: serde_json::Value = serde_json::from_str(
        &db.query_row::<String, _, _>(
            "select target_snapshot_json from note_links where source_block_id='keep-reference'",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(retained["custom"], 1);
    assert_eq!(
        db.query_row::<i64, _, _>(
            "select count(*) from note_links where source_page_id=?1",
            [&saved.page.id],
            |row| row.get(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn late_failure_restores_removed_semantic_links_with_the_whole_note_transaction() {
    let (mut db, saved) = repeated_link_fixture(true);
    let original = state(&db);
    let mut document = saved.document;
    document.blocks.retain(|block| block.id == "keep-reference");
    db.execute_batch("create temp trigger reject_link_reset before update on note_index_state begin select raise(abort, 'late link reset failure'); end;").unwrap();
    let result = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document,
            expected_revision: saved.page.revision,
            save_reason: "manual".into(),
        },
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("late link reset failure")
    );
    assert_eq!(state(&db), original);
}
