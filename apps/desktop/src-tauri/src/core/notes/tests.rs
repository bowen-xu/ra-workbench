use rusqlite::Connection;

use crate::storage::migrate;

use super::{
    document_repository::{load_document_from, save_document_from},
    link_repository::list_backlinks_from,
    model::{
        NoteDocumentBlock, NoteDocumentSaveRequest, NotePageDraft, NotePageMetadataUpdate,
        NotePageMoveRequest, NoteStatusUpdate,
    },
    page_repository::{
        create_page_from, list_pages_from, move_page_from, set_page_status_from, update_page_from,
    },
};

pub(crate) fn migrated_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("migrate database");
    connection
}

#[test]
fn ipc_output_omits_absent_optional_fields() {
    let mut connection = migrated_connection();
    let root = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "根页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("create root page");
    let child = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "子页面".to_owned(),
            parent_page_id: Some(root.page.id.clone()),
            parent_block_position: None,
        },
    )
    .expect("create child page");

    let root_json = serde_json::to_value(&root.page).expect("serialize root page");
    assert_eq!(root_json.get("parentPageId"), None);
    assert_eq!(root_json.get("trashedAt"), None);

    let child_json = serde_json::to_value(&child.page).expect("serialize child page");
    assert_eq!(
        child_json
            .get("parentPageId")
            .and_then(serde_json::Value::as_str),
        Some(root.page.id.as_str())
    );
}

pub(super) fn insert_paper_fixture(connection: &Connection) {
    connection
        .execute(
            "insert into bibliographic_items
         (id, item_type, title, created_at, updated_at)
         values ('paper-1', 'journalArticle', 'Traceable Notes', 'now', 'now')",
            [],
        )
        .expect("insert paper");
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
         values (?1, 'aa/original.pdf', 'application/pdf', 10, 'now')",
            ["a".repeat(64)],
        )
        .expect("insert content");
    connection
        .execute(
            "insert into attachments
         (id, item_id, content_sha256, display_name, attachment_kind, created_at)
         values ('attachment-1', 'paper-1', ?1, 'paper.pdf', 'pdf', 'now')",
            ["a".repeat(64)],
        )
        .expect("insert attachment");
}

#[test]
fn page_document_save_is_versioned_and_atomic() {
    let mut connection = migrated_connection();
    let created = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "研究笔记".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("create page");
    let page_id = created.page.id.clone();
    let document = super::model::NoteDocument {
        title: None,
        schema_version: 1,
        page_id: page_id.clone(),
        blocks: vec![NoteDocumentBlock {
            id: "block-a".to_owned(),
            block_type: "paragraph".to_owned(),
            props: serde_json::json!({}),
            content: serde_json::json!({"kind":"inline","items":[{"type":"text","text":"可恢复内容","styles":[]}]}),
            children: Vec::new(),
        }],
    };
    let saved = save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document: document.clone(),
            expected_revision: 0,
            save_reason: "manual".to_owned(),
        },
    )
    .expect("save document");
    assert_eq!(saved.page.revision, 1);
    assert_eq!(
        load_document_from(&connection, &page_id)
            .expect("reload")
            .document
            .blocks[0]
            .id,
        "block-a"
    );
    let conflict = save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "autosave".to_owned(),
        },
    )
    .expect_err("reject stale save");
    assert!(conflict.to_string().contains("当前版本为 1"));
}

#[test]
fn first_autosave_keeps_a_recoverable_baseline_after_metadata_changes() {
    let mut connection = migrated_connection();
    let created = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "无标题".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("create page");
    let page = update_page_from(
        &mut connection,
        NotePageMetadataUpdate {
            page_id: created.page.id.clone(),
            title: Some("命名后的页面".to_owned()),
            is_favorite: None,
            project_ids: None,
            tag_ids: None,
            expected_revision: 0,
        },
    )
    .expect("update title");
    save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document: created.document,
            expected_revision: page.revision,
            save_reason: "autosave".to_owned(),
        },
    )
    .expect("autosave");
    let revision_count: i64 = connection
        .query_row(
            "select count(*) from note_revisions where page_id = ?1",
            [&page.id],
            |row| row.get(0),
        )
        .expect("revision count");
    assert_eq!(revision_count, 1);
}

#[test]
fn page_tree_rejects_cycles() {
    let mut connection = migrated_connection();
    let parent = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "父页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("parent");
    let child = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "子页面".to_owned(),
            parent_page_id: Some(parent.page.id.clone()),
            parent_block_position: None,
        },
    )
    .expect("child");
    let parent_revision = load_document_from(&connection, &parent.page.id)
        .expect("refreshed parent")
        .page
        .revision;
    let error = move_page_from(
        &mut connection,
        NotePageMoveRequest {
            page_id: parent.page.id,
            parent_page_id: Some(child.page.id),
            position: 0,
            expected_revision: parent_revision,
        },
    )
    .expect_err("reject cycle");
    assert!(error.to_string().contains("环路"));
}

#[test]
fn child_pages_are_transactional_document_blocks() {
    let mut connection = migrated_connection();
    let parent = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "父页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("parent");
    let child = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "子页面".to_owned(),
            parent_page_id: Some(parent.page.id.clone()),
            parent_block_position: None,
        },
    )
    .expect("child");
    let loaded_parent = load_document_from(&connection, &parent.page.id).expect("load parent");
    let child_block = loaded_parent
        .document
        .blocks
        .iter()
        .find(|block| block.block_type == "childPage")
        .expect("child page block");
    assert_eq!(loaded_parent.page.revision, 1);
    assert_eq!(
        child_block
            .content
            .get("targetId")
            .and_then(|value| value.as_str()),
        Some(child.page.id.as_str())
    );

    let renamed = update_page_from(
        &mut connection,
        NotePageMetadataUpdate {
            page_id: child.page.id.clone(),
            title: Some("更新后的子页面".to_owned()),
            is_favorite: None,
            project_ids: None,
            tag_ids: None,
            expected_revision: child.page.revision,
        },
    )
    .expect("rename child");
    let renamed_parent = load_document_from(&connection, &parent.page.id).expect("reload parent");
    assert_eq!(renamed_parent.page.revision, 2);
    assert_eq!(
        renamed_parent
            .document
            .blocks
            .iter()
            .find(|block| block.block_type == "childPage")
            .and_then(|block| block.content.get("labelSnapshot"))
            .and_then(|value| value.as_str()),
        Some("更新后的子页面")
    );

    let trashed = set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: renamed.id.clone(),
            status: "trashed".to_owned(),
            expected_revision: renamed.revision,
        },
    )
    .expect("trash child");
    assert!(
        load_document_from(&connection, &parent.page.id)
            .expect("parent after trash")
            .document
            .blocks
            .iter()
            .all(|block| block.block_type != "childPage")
    );
    set_page_status_from(
        &mut connection,
        NoteStatusUpdate {
            id: trashed.id,
            status: "active".to_owned(),
            expected_revision: trashed.revision,
        },
    )
    .expect("restore child");
    assert!(
        load_document_from(&connection, &parent.page.id)
            .expect("parent after restore")
            .document
            .blocks
            .iter()
            .any(|block| block.block_type == "childPage")
    );
    let restored_parent =
        load_document_from(&connection, &parent.page.id).expect("restored parent");
    let without_child = super::model::NoteDocument {
        title: None,
        schema_version: restored_parent.document.schema_version,
        page_id: restored_parent.document.page_id,
        blocks: restored_parent
            .document
            .blocks
            .into_iter()
            .filter(|block| block.block_type != "childPage")
            .collect(),
    };
    save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document: without_child,
            expected_revision: restored_parent.page.revision,
            save_reason: "structure".to_owned(),
        },
    )
    .expect("remove child block");
    assert_eq!(
        load_document_from(&connection, &child.page.id)
            .expect("trashed child remains recoverable")
            .page
            .status,
        "trashed"
    );
}

#[test]
fn child_page_creation_preserves_the_requested_parent_block_position() {
    let mut connection = migrated_connection();
    let parent = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "父页面".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("parent");
    let trailing = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "末尾子页面".to_owned(),
            parent_page_id: Some(parent.page.id.clone()),
            parent_block_position: None,
        },
    )
    .expect("trailing child");
    let leading = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "光标子页面".to_owned(),
            parent_page_id: Some(parent.page.id.clone()),
            parent_block_position: Some(0),
        },
    )
    .expect("leading child");

    let loaded = load_document_from(&connection, &parent.page.id).expect("load parent");
    assert_eq!(
        loaded
            .document
            .blocks
            .iter()
            .map(|block| (
                block.block_type.as_str(),
                block
                    .content
                    .get("targetId")
                    .and_then(|value| value.as_str()),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("childPage", Some(leading.page.id.as_str())),
            ("paragraph", None),
            ("childPage", Some(trailing.page.id.as_str())),
        ]
    );
    let children = list_pages_from(&connection, None)
        .expect("list pages")
        .into_iter()
        .filter(|page| page.parent_page_id.as_deref() == Some(parent.page.id.as_str()))
        .map(|page| (page.id, page.position, page.revision))
        .collect::<Vec<_>>();
    assert_eq!(
        children,
        vec![(leading.page.id, 0, 0), (trailing.page.id, 1, 1)]
    );
}

#[test]
fn page_move_reorders_siblings_without_duplicate_positions() {
    let mut connection = migrated_connection();
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
    let third = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "三".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("third");
    move_page_from(
        &mut connection,
        NotePageMoveRequest {
            page_id: third.page.id.clone(),
            parent_page_id: None,
            position: 0,
            expected_revision: 0,
        },
    )
    .expect("move to start");
    let pages = list_pages_from(&connection, None).expect("list ordered pages");
    assert_eq!(
        pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        vec!["三", "一", "二"]
    );
    assert_eq!(
        pages.iter().map(|page| page.position).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(
        pages
            .iter()
            .find(|page| page.id == first.page.id)
            .expect("first page")
            .revision,
        1
    );
    assert_eq!(
        pages
            .iter()
            .find(|page| page.id == second.page.id)
            .expect("second page")
            .revision,
        1
    );
}

#[test]
fn document_save_tracks_managed_assets_and_semantic_links() {
    let mut connection = migrated_connection();
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
         values (?1, 'bb/bb/asset', 'image/png', 3, 'now')",
            ["b".repeat(64)],
        )
        .expect("asset object");
    let source = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "Source".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("source");
    let target = create_page_from(
        &mut connection,
        NotePageDraft {
            title: "Target".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .expect("target");
    let document = super::model::NoteDocument {
        title: None,
        schema_version: 1,
        page_id: source.page.id.clone(),
        blocks: vec![
            NoteDocumentBlock {
                id: "asset-block".to_owned(),
                block_type: "image".to_owned(),
                props: serde_json::json!({}),
                content: serde_json::json!({"kind":"asset","assetId":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","displayName":"figure.png","mimeType":"image/png","caption":""}),
                children: vec![],
            },
            NoteDocumentBlock {
                id: "reference-block".to_owned(),
                block_type: "researchObjectReference".to_owned(),
                props: serde_json::json!({}),
                content: serde_json::json!({"kind":"reference","targetKind":"note_page","targetId":target.page.id,"labelSnapshot":"Target","quoteSnapshot":""}),
                children: vec![],
            },
        ],
    };
    save_document_from(
        &mut connection,
        NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".to_owned(),
        },
    )
    .expect("save relations");
    let asset_count: i64 = connection
        .query_row(
            "select count(*) from note_block_assets where block_id = 'asset-block'",
            [],
            |row| row.get(0),
        )
        .expect("asset relation");
    assert_eq!(asset_count, 1);
    assert_eq!(
        list_backlinks_from(&connection, "note_page", &target.page.id)
            .expect("backlinks")
            .len(),
        1
    );
}

pub(crate) fn insert_reading_fixture(db: &Connection, alias: &str, body: &str) -> String {
    let page_id = uuid::Uuid::new_v4().to_string();
    db.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Retained paper','now','now')", [&page_id]).unwrap();
    db.execute(
        "insert into reading_note_page_map(legacy_id,page_id) values(?1,?2)",
        rusqlite::params![alias, page_id],
    )
    .unwrap();
    db.execute("insert into note_blocks(id,scope_kind,page_id,block_type,content_json,plain_text,created_at,updated_at) values(?1,'page',?2,'paragraph',?3,?4,'now','now')", rusqlite::params![uuid::Uuid::new_v4().to_string(),page_id,serde_json::json!({"kind":"inline","items":[{"type":"text","text":body,"styles":[]}]}).to_string(),body]).unwrap();
    db.execute("insert into reading_note_sources(block_id,page_id,note_type,color,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at) values(?1,?2,'annotation','#f4cf65','missing-paper','Retained paper title','missing-attachment','',?3,2,'[]','Original quoted evidence','Before','After',1,'now','now')", rusqlite::params![alias,page_id,"a".repeat(64)]).unwrap();
    page_id
}
