use rusqlite::Connection;

use super::{
    link_repository::{list_backlinks_from, upsert_link_from},
    model::{
        NoteLinkDraft, NoteLinkTarget, NotePageDraft, NoteSearchQuery, NoteStatusUpdate,
        ReadingNoteDraft, ReadingNoteUpdate, SourceRect, SourceSelectorSegment, SourceSelectorV2,
    },
    page_repository::create_page_from,
    reading_note_repository::{
        create_reading_note_from, reading_note_from, set_reading_note_status_from,
        update_reading_note_from,
    },
    search::{rebuild_note_index_from, search_notes_from},
    tests::{insert_paper_fixture, migrated_connection},
};

#[test]
fn reading_note_survives_source_deletion_and_shares_one_lifecycle() {
    let mut connection = migrated_connection();
    insert_paper_fixture(&connection);
    let note = create_reading_note_from(
        &mut connection,
        ReadingNoteDraft {
            id: Some("reading-1".to_owned()),
            source_anchor_id: None,
            note_type: "annotation".to_owned(),
            body: "重要批注".to_owned(),
            color: None,
            item_id: "paper-1".to_owned(),
            attachment_id: "attachment-1".to_owned(),
            page_index: 2,
            page_label: Some("3".to_owned()),
            rects: vec![SourceRect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            }],
            quote_exact: "quoted\nsource".to_owned(),
            quote_prefix: None,
            quote_suffix: None,
            selector: Some(SourceSelectorV2 {
                schema_version: 2,
                anchor_id: None,
                attachment_sha256: Some("a".repeat(64)),
                segments: vec![
                    SourceSelectorSegment {
                        page_index: 2,
                        page_label: Some("3".to_owned()),
                        rects: vec![SourceRect {
                            x: 1.0,
                            y: 2.0,
                            width: 3.0,
                            height: 4.0,
                        }],
                        exact_text: "quoted".to_owned(),
                        prefix: Some("before".to_owned()),
                        suffix: Some(String::new()),
                    },
                    SourceSelectorSegment {
                        page_index: 3,
                        page_label: Some("4".to_owned()),
                        rects: vec![SourceRect {
                            x: 1.0,
                            y: 1.0,
                            width: 2.0,
                            height: 1.0,
                        }],
                        exact_text: "source".to_owned(),
                        prefix: Some(String::new()),
                        suffix: Some("after".to_owned()),
                    },
                ],
            }),
        },
    )
    .expect("create reading note");
    assert_eq!(note.source.state, "exact");
    assert_eq!(
        note.source
            .selector
            .as_ref()
            .map(|selector| selector.segments.len()),
        Some(2)
    );
    let selector_versions: (i64, i64) = connection
        .query_row(
            "select a.selector_version, s.locator_version
             from source_anchors a join reading_note_sources s on s.current_anchor_id = a.id
             where s.block_id = ?1",
            [&note.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("selector versions");
    assert_eq!(selector_versions, (2, 2));
    let source_anchor_id = note.source.anchor_id.clone().expect("source anchor");
    let derived_note = create_reading_note_from(
        &mut connection,
        ReadingNoteDraft {
            id: Some("reading-2".to_owned()),
            source_anchor_id: Some(source_anchor_id.clone()),
            note_type: "annotation".to_owned(),
            body: "从既有来源形成的人工笔记".to_owned(),
            color: None,
            item_id: "paper-1".to_owned(),
            attachment_id: "attachment-1".to_owned(),
            page_index: 2,
            page_label: None,
            rects: vec![],
            quote_exact: "调用方快照不会覆盖锚点".to_owned(),
            quote_prefix: None,
            quote_suffix: None,
            selector: None,
        },
    )
    .expect("reuse source anchor");
    assert_eq!(
        derived_note.source.anchor_id.as_deref(),
        Some(source_anchor_id.as_str())
    );
    assert_eq!(derived_note.source.quote_exact, "quoted\nsource");
    let source_count: i64 = connection
        .query_row("select count(*) from source_anchors", [], |row| row.get(0))
        .expect("count anchors");
    assert_eq!(source_count, 1);
    let updated = update_reading_note_from(
        &mut connection,
        ReadingNoteUpdate {
            id: note.id.clone(),
            color: None,
            position: None,
            is_favorite: Some(false),
            project_ids: None,
            tag_ids: None,
            expected_revision: 0,
        },
    )
    .expect("update note");
    assert_eq!(updated.revision, 1);
    let trashed = set_reading_note_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: note.id.clone(),
            status: "trashed".to_owned(),
            expected_revision: 1,
        },
    )
    .expect("trash note");
    assert_eq!(trashed.status, "trashed");
    connection
        .execute("delete from bibliographic_items where id = 'paper-1'", [])
        .expect("delete paper");
    let orphan = reading_note_from(&connection, &note.id).expect("read orphan");
    assert_eq!(orphan.source.state, "unavailable");
    assert_eq!(orphan.source.quote_exact, "quoted\nsource");
}

#[test]
fn links_backlinks_and_search_are_rebuildable() {
    let mut connection = migrated_connection();
    let source = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "来源页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("source");
    let target = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "目标页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("target");
    upsert_link_from(
        &connection,
        NoteLinkDraft {
            source_page_id: source.page.id.clone(),
            source_block_id: None,
            target: NoteLinkTarget {
                kind: "note_page".to_owned(),
                id: target.page.id.clone(),
                label_snapshot: target.page.title.clone(),
                quote_snapshot: None,
            },
            relation: "reference".to_owned(),
        },
    )
    .expect("link");
    assert_eq!(
        list_backlinks_from(&connection, "note_page", &target.page.id)
            .expect("backlinks")
            .len(),
        1
    );
    rebuild_note_index_from(&mut connection).expect("rebuild index");
    let result = search_notes_from(
        &connection,
        NoteSearchQuery {
            search: Some("目标".to_owned()),
            status: None,
            object_kinds: None,
            project_id: None,
            tag_id: None,
            paper_id: None,
            note_type: None,
            source_state: None,
            is_favorite: None,
            limit: Some(10),
            cursor: None,
        },
    )
    .expect("search");
    assert_eq!(result.items[0].object_id, target.page.id);
}

#[test]
fn legacy_annotation_migration_preserves_ids_and_source_snapshot() {
    let connection = Connection::open_in_memory().expect("open legacy database");
    connection.execute_batch(
        "pragma foreign_keys = on;
         create table research_projects(id text primary key, title text not null);
         create table tags(id text primary key, name text not null);
         create table content_objects(sha256 text primary key, relative_path text not null, mime_type text not null, byte_size integer not null, created_at text not null);
         create table bibliographic_items(id text primary key, title text not null, trashed_at text);
         create table attachments(id text primary key, item_id text, content_sha256 text, display_name text);
         create table source_anchors(id text primary key, item_id text, attachment_id text, content_sha256 text, selector_version integer, page_index integer, page_label text, rects_json text, quote_exact text, quote_prefix text, quote_suffix text, created_at text);
         create table annotations(id text primary key, anchor_id text, annotation_type text, color text, body text, created_at text, updated_at text);
         insert into bibliographic_items values ('paper', 'Legacy Paper', null);
         insert into content_objects values ('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'a.pdf', 'application/pdf', 1, 'now');
         insert into attachments values ('attachment', 'paper', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'legacy.pdf');
         insert into source_anchors values ('anchor', 'paper', 'attachment', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1, 4, '5', '[]', 'quote', '', '', 'now');
         insert into annotations values ('legacy-note', 'anchor', 'note', '#fff000', 'body', 'now', 'now');",
    ).expect("legacy schema");
    connection
        .execute_batch(include_str!("../../../migrations/0012_notes_core.sql"))
        .expect("core notes migration");
    connection
        .execute_batch(include_str!(
            "../../../migrations/0013_migrate_annotations_to_reading_notes.sql"
        ))
        .expect("annotation migration");
    let migrated: (String, String, String) = connection.query_row(
        "select b.id, b.block_type, s.quote_exact from note_blocks b join reading_note_sources s on s.block_id = b.id",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).expect("migrated row");
    assert_eq!(
        migrated,
        (
            "legacy-note".to_owned(),
            "reading_annotation".to_owned(),
            "quote".to_owned()
        )
    );
    let counts: (i64, i64) = connection.query_row(
        "select (select count(*) from annotations), (select count(*) from reading_note_sources)",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).expect("migration counts");
    assert_eq!(counts, (1, 1));
}

#[test]
fn reading_document_upgrade_preserves_literal_text_identity_properties_and_old_links() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 142);
    db.execute_batch("insert into note_pages(id,title,created_at,updated_at) values('collision','Ordinary page','now','now');
      insert into tags(id,name) values('tag','Migration tag');
      insert into research_projects(id,title,summary,created_at) values('project','Migration project','','now');
      insert into note_blocks(id,scope_kind,block_type,content_json,props_json,status,is_favorite,revision,created_at,updated_at,trashed_at)
      values('collision','reading','reading_annotation','{\"body\":\"# Literal\\n\\n**text**\\n\"}','{\"color\":\"#112233\"}','trashed',1,7,'created','modified','deleted');
      insert into note_blocks(id,scope_kind,block_type,content_json,created_at,updated_at)
      values('empty','reading','reading_page_note','{\"body\":\"\"}','created','modified');
      insert into reading_note_tags(block_id,tag_id) values('collision','tag');
      insert into project_reading_notes(block_id,project_id,added_at) values('collision','project','linked');
      insert into note_links(id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at)
      values('old-link','collision','reading_note','collision','reference','{\"labelSnapshot\":\"Retained\",\"quoteSnapshot\":\"Quote\"}','created');").unwrap();
    for alias in ["collision", "empty"] {
        db.execute("insert into reading_note_sources(block_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,content_sha256,page_index,quote_exact,created_at,updated_at) values(?1,'deleted-paper','Retained paper','deleted-pdf',?2,3,'Quote','created','modified')",rusqlite::params![alias,"a".repeat(64)]).unwrap();
    }
    crate::storage::migrate(&mut db).unwrap();
    let note = reading_note_from(&db, "collision").unwrap();
    assert_ne!(note.page_id, "collision");
    assert_eq!(note.body, "# Literal\n\n**text**\n");
    assert_eq!(note.source.state, "unavailable");
    assert_eq!(note.source.quote_exact, "Quote");
    assert_eq!(note.color, "#112233");
    assert_eq!(note.status, "trashed");
    assert_eq!(note.revision, 7);
    assert!(note.is_favorite);
    assert_eq!(note.created_at, "created");
    assert_eq!(note.updated_at, "modified");
    assert_eq!(note.trashed_at.as_deref(), Some("deleted"));
    assert_eq!(note.tag_ids, vec!["tag"]);
    assert_eq!(note.project_ids, vec!["project"]);
    let snapshot = super::document_repository::load_document_from(&db, &note.page_id).unwrap();
    assert!(
        snapshot
            .document
            .blocks
            .iter()
            .all(|block| block.block_type == "paragraph")
    );
    assert_eq!(snapshot.document.blocks.len(), 4);
    assert_eq!(
        snapshot.document.blocks[0].content["items"][0]["styles"],
        serde_json::json!([])
    );
    let empty = reading_note_from(&db, "empty").unwrap();
    assert!(
        super::document_repository::load_document_from(&db, &empty.page_id)
            .unwrap()
            .document
            .blocks
            .is_empty()
    );
    assert!(!list_backlinks_from(&db, "reading_note", "collision").unwrap()[0].is_broken);
    assert_eq!(
        db.query_row(
            "select title from note_pages where id='collision'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "Ordinary page"
    );
    let identity = note.page_id;
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        reading_note_from(&db, "collision").unwrap().page_id,
        identity
    );
    assert_eq!(
        db.query_row("select count(*) from reading_note_page_map", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "select count(*) from note_blocks where scope_kind='reading'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn position_annotation_moves_atomically_without_a_quote_or_duplicate_document() {
    let mut connection = migrated_connection();
    insert_paper_fixture(&connection);
    let rect = SourceRect {
        x: 0.2,
        y: 0.3,
        width: 0.00001,
        height: 0.00001,
    };
    let note = create_reading_note_from(
        &mut connection,
        ReadingNoteDraft {
            id: None,
            source_anchor_id: None,
            note_type: "annotation".into(),
            body: "Sticky note body".into(),
            color: None,
            item_id: "paper-1".into(),
            attachment_id: "attachment-1".into(),
            page_index: 2,
            page_label: Some("iii".into()),
            rects: vec![rect.clone()],
            quote_exact: String::new(),
            quote_prefix: None,
            quote_suffix: None,
            selector: Some(SourceSelectorV2 {
                schema_version: 2,
                anchor_id: None,
                attachment_sha256: Some("a".repeat(64)),
                segments: vec![SourceSelectorSegment {
                    page_index: 2,
                    page_label: Some("iii".into()),
                    rects: vec![rect],
                    exact_text: String::new(),
                    prefix: None,
                    suffix: None,
                }],
            }),
        },
    )
    .unwrap();
    assert_eq!(note.source.state, "exact");
    let request = |x: f64, revision: i64| {
        serde_json::from_value::<ReadingNoteUpdate>(
            serde_json::json!({ "id": note.id, "expectedRevision": revision,
        "position": { "x": x, "y": 0.7, "width": 0.00001, "height": 0.00001 } }),
        )
        .unwrap()
    };
    let moved = update_reading_note_from(&mut connection, request(0.6, note.revision)).unwrap();
    assert_eq!(moved.page_id, note.page_id);
    assert_eq!(moved.source.anchor_id, note.source.anchor_id);
    assert_eq!(moved.source.quote_exact, "");
    assert_eq!(moved.source.rects[0]["x"], 0.6);
    assert_eq!(
        moved.source.selector.as_ref().unwrap().segments[0].rects[0].x,
        0.6
    );
    assert_eq!(moved.body, "Sticky note body");
    assert!(update_reading_note_from(&mut connection, request(1.1, moved.revision)).is_err());
    assert!(update_reading_note_from(&mut connection, request(0.8, note.revision)).is_err());
    assert_eq!(
        reading_note_from(&connection, &note.id).unwrap().revision,
        moved.revision
    );
    let anchor_rects: String = connection
        .query_row(
            "select rects_json from source_anchors where id=?1",
            [moved.source.anchor_id.as_ref().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&anchor_rects).unwrap()[0]["x"],
        0.6
    );
    let trashed = set_reading_note_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: note.id.clone(),
            status: "trashed".into(),
            expected_revision: moved.revision,
        },
    )
    .unwrap();
    let restored = set_reading_note_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: note.id,
            status: "active".into(),
            expected_revision: trashed.revision,
        },
    )
    .unwrap();
    assert_eq!(restored.source.rects, moved.source.rects);
}
