use std::fs;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage::migrate;

use super::{
    document_repository::{load_document_from, save_document_from},
    lifecycle::{permanently_delete_page_from, permanently_delete_reading_note_from},
    link_repository::{list_backlinks_from, upsert_link_from},
    model::{
        NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest, NoteLinkDraft, NoteLinkTarget,
        NotePageDraft, NotePermanentDeleteRequest, NoteStatusUpdate, ReadingNoteDraft,
    },
    page_repository::{create_page_from, list_pages_from, set_page_status_from},
    reading_note_repository::{
        create_reading_note_from, reading_note_from, set_reading_note_status_from,
    },
};

fn migrated_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("migrate database");
    connection
}

fn insert_paper_fixture(connection: &Connection) {
    connection.execute("insert into bibliographic_items (id, item_type, title, created_at, updated_at) values ('paper-1', 'journalArticle', 'Traceable Notes', 'now', 'now')", []).expect("paper");
    connection.execute("insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at) values (?1, 'aa/original.pdf', 'application/pdf', 10, 'now')", ["a".repeat(64)]).expect("content");
    connection.execute("insert into attachments (id, item_id, content_sha256, display_name, attachment_kind, created_at) values ('attachment-1', 'paper-1', ?1, 'paper.pdf', 'pdf', 'now')", ["a".repeat(64)]).expect("attachment");
}

#[test]
fn permanent_page_delete_requires_trash_and_preserves_broken_backlink_snapshot() {
    let mut connection = migrated_connection();
    let source = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "来源".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("source");
    let target = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "目标".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("target");
    let child = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "子页面".to_owned(),
            parent_page_id: Some(target.page.id.clone()),
            parent_block_position: None,
        },
    )
    .expect("child");
    upsert_link_from(
        &connection,
        NoteLinkDraft {
            source_page_id: source.page.id.clone(),
            source_block_id: None,
            target: NoteLinkTarget {
                kind: "note_page".to_owned(),
                id: target.page.id.clone(),
                label_snapshot: "目标快照".to_owned(),
                quote_snapshot: None,
            },
            relation: "reference".to_owned(),
        },
    )
    .expect("link");
    let target_revision = load_document_from(&connection, &target.page.id)
        .expect("refreshed target")
        .page
        .revision;
    let error = permanently_delete_page_from(
        &mut connection,
        std::path::Path::new("/tmp/unused-note-assets"),
        NotePermanentDeleteRequest {
            id: target.page.id.clone(),
            expected_revision: target_revision,
        },
    )
    .expect_err("active page cannot be purged");
    assert!(error.to_string().contains("回收站"));
    let trashed = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: target.page.id.clone(),
            status: "trashed".to_owned(),
            expected_revision: target_revision,
        },
    )
    .expect("trash target");
    permanently_delete_page_from(
        &mut connection,
        std::path::Path::new("/tmp/unused-note-assets"),
        NotePermanentDeleteRequest {
            id: target.page.id.clone(),
            expected_revision: trashed.revision,
        },
    )
    .expect("purge target");
    let backlink = list_backlinks_from(&connection, "note_page", &target.page.id)
        .expect("broken backlink")
        .remove(0);
    assert!(backlink.is_broken);
    assert_eq!(backlink.target.label_snapshot, "目标快照");
    let child = list_pages_from(&connection, None)
        .expect("remaining pages")
        .into_iter()
        .find(|page| page.id == child.page.id)
        .expect("child retained");
    assert_eq!(child.parent_page_id, None);
}

#[test]
fn permanent_reading_note_delete_preserves_broken_reference_snapshot() {
    let mut connection = migrated_connection();
    insert_paper_fixture(&connection);
    let source = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "来源页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("source page");
    let note = create_reading_note_from(
        &mut connection,
        ReadingNoteDraft {
            id: Some("reading-delete".to_owned()),
            source_anchor_id: None,
            note_type: "highlight".to_owned(),
            body: String::new(),
            color: None,
            item_id: "paper-1".to_owned(),
            attachment_id: "attachment-1".to_owned(),
            page_index: 0,
            page_label: None,
            rects: vec![],
            quote_exact: "保留的引用快照".to_owned(),
            quote_prefix: None,
            quote_suffix: None,
            selector: None,
        },
    )
    .expect("reading note");
    assert!(note.source.selector.is_none());
    let anchor_id = note.source.anchor_id.clone().expect("source anchor");
    upsert_link_from(
        &connection,
        NoteLinkDraft {
            source_page_id: source.page.id,
            source_block_id: None,
            target: NoteLinkTarget {
                kind: "reading_note".to_owned(),
                id: note.id.clone(),
                label_snapshot: "论文".to_owned(),
                quote_snapshot: Some(note.source.quote_exact.clone()),
            },
            relation: "reference".to_owned(),
        },
    )
    .expect("reading reference");
    let trashed = set_reading_note_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: note.id.clone(),
            status: "trashed".to_owned(),
            expected_revision: 0,
        },
    )
    .expect("trash note");
    permanently_delete_reading_note_from(
        &mut connection,
        std::path::Path::new("/tmp/unused-note-assets"),
        NotePermanentDeleteRequest {
            id: note.id.clone(),
            expected_revision: trashed.revision,
        },
    )
    .expect("purge note");
    assert!(reading_note_from(&connection, &note.id).is_err());
    let anchor_count: i64 = connection
        .query_row(
            "select count(*) from source_anchors where id = ?1",
            [&anchor_id],
            |row| row.get(0),
        )
        .expect("anchor count");
    assert_eq!(anchor_count, 0);
    let backlink = list_backlinks_from(&connection, "reading_note", &note.id)
        .expect("broken reference")
        .remove(0);
    assert!(backlink.is_broken);
    assert_eq!(
        backlink.target.quote_snapshot.as_deref(),
        Some("保留的引用快照")
    );
}

#[test]
fn permanent_page_delete_cleans_only_unshared_managed_assets() {
    let mut connection = migrated_connection();
    let content_directory =
        std::env::temp_dir().join(format!("research-assistant-note-assets-{}", Uuid::new_v4()));
    let sha256 = "b".repeat(64);
    let relative_path = format!("bb/bb/{sha256}");
    let asset_path = content_directory.join(&relative_path);
    fs::create_dir_all(asset_path.parent().expect("asset parent")).expect("asset directory");
    fs::write(&asset_path, b"asset").expect("asset file");
    connection.execute(
        "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at) values (?1, ?2, 'image/png', 5, 'now')",
        rusqlite::params![sha256, relative_path],
    ).expect("asset object");
    let first = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "一".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("first");
    let second = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "二".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("second");
    for (page_id, block_id) in [
        (&first.page.id, "asset-one"),
        (&second.page.id, "asset-two"),
    ] {
        save_document_from(&mut connection, NoteDocumentSaveRequest {
            document: NoteDocument { title: None, schema_version: 1, page_id: page_id.clone(), blocks: vec![NoteDocumentBlock {
                id: block_id.to_owned(), block_type: "image".to_owned(), props: serde_json::json!({}),
                content: serde_json::json!({"kind":"asset","assetId":sha256,"displayName":"figure.png","mimeType":"image/png","caption":""}), children: vec![],
            }] },
            expected_revision: 0, save_reason: "manual".to_owned(),
        }).expect("save asset document");
    }
    let first = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: first.page.id,
            status: "trashed".to_owned(),
            expected_revision: 1,
        },
    )
    .expect("trash first");
    let report = permanently_delete_page_from(
        &mut connection,
        &content_directory,
        NotePermanentDeleteRequest {
            id: first.id,
            expected_revision: first.revision,
        },
    )
    .expect("delete first");
    assert_eq!(report.scheduled_asset_count, 0);
    assert!(asset_path.exists());
    let current_second = list_pages_from(&connection, None)
        .expect("remaining page")
        .into_iter()
        .find(|page| page.id == second.page.id)
        .expect("second page");
    let second = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: second.page.id,
            status: "trashed".to_owned(),
            expected_revision: current_second.revision,
        },
    )
    .expect("trash second");
    let report = permanently_delete_page_from(
        &mut connection,
        &content_directory,
        NotePermanentDeleteRequest {
            id: second.id,
            expected_revision: second.revision,
        },
    )
    .expect("delete second");
    assert_eq!(report.scheduled_asset_count, 1);
    assert!(asset_path.exists());
    fs::remove_dir_all(&content_directory).expect("remove test directory");
}

fn insert_preserved_asset(
    connection: &Connection,
    directory: &std::path::Path,
    bytes: &[u8],
) -> String {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let relative_path = format!("{}/{}/{}", &sha256[..2], &sha256[2..4], sha256);
    let path = directory.join(&relative_path);
    fs::create_dir_all(path.parent().expect("asset parent")).expect("asset directory");
    fs::write(path, bytes).expect("asset bytes");
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
         values (?1, ?2, 'image/png', ?3, 'now')",
            rusqlite::params![sha256, relative_path, bytes.len() as i64],
        )
        .expect("asset object");
    sha256
}

fn preserve_page_assets(connection: &mut Connection, page_id: &str, assets: &[&str]) -> String {
    let observed = super::hbs::read(connection, page_id).expect("authoritative document");
    let mut content = observed["saved"].clone();
    content["blocks"] =
        serde_json::json!(assets.iter().enumerate().map(|(index, asset)| {
        serde_json::json!({"id":format!("preserved-{index}"), "type":"image", "props":{},
            "content":{"kind":"asset", "assetId":asset, "displayName":"screenshot.png",
                "mimeType":"image/png", "caption":""}, "children":[]})
    }).collect::<Vec<_>>());
    crate::hbs_storage::test_sources::preservation_save(
        connection,
        "note",
        page_id,
        serde_json::json!({
            "format":1, "kind":"note", "observed":observed, "content":content,
            "ancestor":observed["saved"], "baselineAncestor":observed["baseline"], "ordinary":[],
        }),
    )
    .expect("preserve unsaved images");
    connection
        .query_row(
            "select checkpoint_json from hbs_preservations where kind='note' and resource_id=?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("preservation record")
}

fn has_content_reference(connection: &Connection, sha256: &str) -> bool {
    connection
        .query_row(
            "select exists(select 1 from content_object_references where content_sha256=?1)",
            [sha256],
            |row| row.get(0),
        )
        .expect("content references")
}

#[test]
fn permanent_page_delete_releases_preservation_only_assets_after_restore_and_keeps_shared_assets() {
    let mut connection = migrated_connection();
    let directory = tempfile::tempdir().expect("asset directory");
    let unique = insert_preserved_asset(&connection, directory.path(), b"unique screenshot");
    let shared = insert_preserved_asset(&connection, directory.path(), b"shared screenshot");
    let first = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "first".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("first page");
    let second = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "second".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("second page");
    let checkpoint = preserve_page_assets(&mut connection, &first.page.id, &[&unique, &shared]);
    preserve_page_assets(&mut connection, &second.page.id, &[&shared]);
    let live_assets: i64 = connection
        .query_row("select count(*) from note_block_assets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(live_assets, 0, "these images exist only in durable S-prime");

    let mut page = first.page;
    for status in ["trashed", "active", "trashed"] {
        page = set_page_status_from(
            &mut connection,
            NoteStatusUpdate {
                id: page.id,
                status: status.into(),
                expected_revision: page.revision,
            },
        )
        .expect("trash/restore page");
        let retained: String = connection.query_row(
            "select checkpoint_json from hbs_preservations where kind='note' and resource_id=?1",
            [&page.id], |row| row.get(0),
        ).expect("trash and restore retain preservation");
        assert_eq!(retained, checkpoint);
        assert!(has_content_reference(&connection, &unique));
    }
    let report = permanently_delete_page_from(
        &mut connection,
        directory.path(),
        NotePermanentDeleteRequest {
            id: page.id.clone(),
            expected_revision: page.revision,
        },
    )
    .expect("permanent first page deletion");
    assert_eq!(report.scheduled_asset_count, 1);
    assert_eq!(
        report.estimated_reclaimable_bytes,
        b"unique screenshot".len() as i64
    );
    assert!(!has_content_reference(&connection, &unique));
    assert!(has_content_reference(&connection, &shared));
    let (removed_preservation, remaining_shared_candidate): (i64, i64) = connection
        .query_row(
            "select (select count(*) from hbs_preservations where kind='note' and resource_id=?1),
                (select count(*) from content_object_gc_candidates where content_sha256=?2)",
            rusqlite::params![page.id, shared],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((removed_preservation, remaining_shared_candidate), (0, 0));

    let second = load_document_from(&connection, &second.page.id)
        .expect("second page after root positions changed")
        .page;
    let second = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: second.id,
            status: "trashed".into(),
            expected_revision: second.revision,
        },
    )
    .expect("trash second page");
    let report = permanently_delete_page_from(
        &mut connection,
        directory.path(),
        NotePermanentDeleteRequest {
            id: second.id,
            expected_revision: second.revision,
        },
    )
    .expect("permanent second page deletion");
    assert_eq!(report.scheduled_asset_count, 1);
    assert!(!has_content_reference(&connection, &shared));
    for asset in [&unique, &shared] {
        let (remaining_days, reason): (f64, String) = connection.query_row(
            "select julianday(not_before)-julianday('now'),reason from content_object_gc_candidates where content_sha256=?1",
            [asset], |row| Ok((row.get(0)?, row.get(1)?)),
        ).expect("delayed GC candidate");
        assert!(remaining_days > 29.0 && remaining_days <= 30.0);
        assert_eq!(reason, "live_reference_removed");
        assert!(
            directory
                .path()
                .join(format!("{}/{}/{}", &asset[..2], &asset[2..4], asset))
                .exists(),
            "permanent note deletion schedules GC without deleting image bytes"
        );
    }
}

#[test]
fn failed_permanent_page_delete_rolls_back_preservation_and_asset_pins() {
    let mut connection = migrated_connection();
    let directory = tempfile::tempdir().expect("asset directory");
    let asset = insert_preserved_asset(&connection, directory.path(), b"recoverable screenshot");
    let page = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "recoverable".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("page");
    let checkpoint = preserve_page_assets(&mut connection, &page.page.id, &[&asset]);
    let page = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: page.page.id,
            status: "trashed".into(),
            expected_revision: page.page.revision,
        },
    )
    .expect("trash page");
    connection
        .execute_batch(
            "create trigger reject_note_purge before delete on note_pages
        begin select raise(abort,'synthetic permanent delete failure'); end;",
        )
        .unwrap();
    permanently_delete_page_from(
        &mut connection,
        directory.path(),
        NotePermanentDeleteRequest {
            id: page.id.clone(),
            expected_revision: page.revision,
        },
    )
    .expect_err("failed owner deletion");
    let retained: String = connection
        .query_row(
            "select checkpoint_json from hbs_preservations where kind='note' and resource_id=?1",
            [&page.id],
            |row| row.get(0),
        )
        .expect("original preservation");
    assert_eq!(retained, checkpoint);
    assert!(has_content_reference(&connection, &asset));
    let candidate_count: i64 = connection
        .query_row(
            "select count(*) from content_object_gc_candidates where content_sha256=?1",
            [&asset],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(candidate_count, 0);
    assert_eq!(
        load_document_from(&connection, &page.id)
            .unwrap()
            .page
            .status,
        "trashed"
    );
}
