use super::*;
use crate::notes::{model::ReadingNoteDraft, reading_note_repository::create_reading_note_from};
use rusqlite::StatementStatus;

pub(super) fn query() -> PageCatalogQuery {
    PageCatalogQuery {
        category: "all".to_owned(),
        view: "recent".to_owned(),
        limit: Some(20),
        cursor: None,
        search: None,
        sort: None,
    }
}
fn seed(connection: &Connection, count: usize) {
    for index in 0..count {
        connection.execute("insert into note_pages(id,title,position,created_at,updated_at) values(?1,?2,?3,'2026-10-23T00:00:00Z',?4)",
            rusqlite::params![format!("page-{index:05}"),format!("笔记 {index}"),index as i64,format!("2026-10-23T00:{:02}:{:02}Z",index/60,index%60)]).unwrap();
    }
}
pub(super) fn reading(connection: &mut Connection, kind: &str, quote: &str) -> String {
    let value = create_reading_note_from(
        connection,
        ReadingNoteDraft {
            id: None,
            source_anchor_id: None,
            note_type: kind.to_owned(),
            body: "\n 第一行标题\n正文保持原文".to_owned(),
            color: None,
            item_id: "paper-1".to_owned(),
            attachment_id: "attachment-1".to_owned(),
            page_index: 0,
            page_label: None,
            rects: vec![],
            quote_exact: quote.to_owned(),
            quote_prefix: None,
            quote_suffix: None,
            selector: None,
        },
    )
    .unwrap();
    value.page_id
}

#[test]
fn catalog_upgrade_from_192_preserves_bodies_and_reopens() {
    let path = std::env::temp_dir().join(format!(
        "note-catalog-upgrade-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut connection = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 192);
    seed(&connection, 30);
    crate::notes::tests::insert_paper_fixture(&connection);
    let reading_id = reading(&mut connection, "highlight", "原始摘录");
    let before: String = connection
        .query_row(
            "select group_concat(plain_text,'|') from note_blocks",
            [],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    let result = query_from(&connection, query()).unwrap();
    assert_eq!(result.items.len(), 20);
    let header = query_from(
        &connection,
        PageCatalogQuery {
            category: "dependent".to_owned(),
            ..query()
        },
    )
    .unwrap();
    assert_eq!(header.items.len(), 1);
    assert_eq!(header.items[0].title, "第一行标题");
    assert_eq!(header.items[0].id, format!("page:{reading_id}"));
    assert!(
        matches!(&header.items[0].target,PageCatalogTarget::Reading{page_id,..} if page_id==&reading_id)
    );
    let after: String = connection
        .query_row(
            "select group_concat(plain_text,'|') from note_blocks",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    let old_count: i64 = connection
        .query_row(
            "select count(*) from schema_migrations where version<=192",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_count, 192);
    drop(connection);
    let mut connection = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    assert_eq!(
        query_from(&connection, query()).unwrap().items[0].id,
        result.items[0].id
    );
    assert_eq!(
        connection
            .query_row("pragma integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(
        !connection
            .prepare("pragma foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn catalog_recent_uses_an_index_and_reads_only_a_page_of_headers() {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 192);
    seed(&connection, 2400);
    crate::storage::migrate(&mut connection).unwrap();
    let result = query_from(&connection, query()).unwrap();
    assert_eq!(result.items.len(), 20);
    assert_eq!(result.items[0].id, "page:page-02399");
    assert!(result.items.iter().all(|item| item.snippet.is_empty()));
    let next = query_from(
        &connection,
        PageCatalogQuery {
            cursor: result.next_cursor,
            ..query()
        },
    )
    .unwrap();
    assert_eq!(next.items[0].id, "page:page-02379");
    for category in [false, true] {
        let sql = query_sql(false, category);
        assert!(!sql.contains("note_blocks"));
        assert!(!sql.contains("note_search"));
        let mut statement = connection.prepare(&sql).unwrap();
        statement.raw_bind_parameter(1, "active").unwrap();
        statement.raw_bind_parameter(2, i64::MAX).unwrap();
        statement.raw_bind_parameter(3, "").unwrap();
        statement.raw_bind_parameter(4, 21).unwrap();
        if category {
            statement.raw_bind_parameter(5, "independent").unwrap();
        }
        let mut count = 0;
        let mut rows = statement.raw_query();
        while rows.next().unwrap().is_some() {
            count += 1;
        }
        drop(rows);
        assert_eq!(count, 21);
        assert_eq!(statement.get_status(StatementStatus::FullscanStep), 0);
        assert!(statement.get_status(StatementStatus::VmStep) < 1000);
        let plan = connection
            .prepare(&format!("explain query plan {sql}"))
            .unwrap()
            .query_map(
                rusqlite::params_from_iter(
                    (0..if category { 5 } else { 4 }).map(|_| rusqlite::types::Value::Null),
                ),
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join(";");
        assert!(
            plan.contains(if category {
                "note_catalog_category_recent_utc"
            } else {
                "note_catalog_recent_utc"
            }),
            "{plan}"
        );
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
}

#[test]
fn catalog_headers_track_metadata_sources_favorites_and_body_title_atomically() {
    let mut connection = crate::notes::tests::migrated_connection();
    seed(&connection, 2);
    crate::notes::tests::insert_paper_fixture(&connection);
    let dependent = reading(&mut connection, "annotation", "引用");
    let independent = reading(&mut connection, "page_note", "");
    let page = query_from(&connection, query()).unwrap();
    assert_eq!(page.items.len(), 4);
    assert_eq!(
        query_from(
            &connection,
            PageCatalogQuery {
                category: "independent".to_owned(),
                ..query()
            }
        )
        .unwrap()
        .items
        .len(),
        3
    );
    connection
        .execute(
            "update note_pages set is_favorite=1 where id=?1",
            [&independent],
        )
        .unwrap();
    let favorites = query_from(
        &connection,
        PageCatalogQuery {
            view: "favorites".to_owned(),
            ..query()
        },
    )
    .unwrap();
    assert_eq!(favorites.items.len(), 1);
    assert_eq!(favorites.items[0].id, format!("page:{independent}"));
    connection
        .execute(
            "update note_blocks set plain_text='新标题',content_json=?2 where page_id=?1",
            rusqlite::params![&dependent,serde_json::json!({"kind":"inline","items":[{"type":"text","text":"新标题","styles":[]}]}).to_string()],
        )
        .unwrap();
    connection
        .execute(
            "update note_pages set revision=revision+1 where id=?1",
            [&dependent],
        )
        .unwrap();
    let header = query_from(
        &connection,
        PageCatalogQuery {
            category: "dependent".to_owned(),
            ..query()
        },
    )
    .unwrap();
    assert_eq!(header.items[0].title, "新标题");
    let tx = connection.transaction().unwrap();
    tx.execute(
        "update note_pages set title='不会提交',status='trashed' where id='page-00000'",
        [],
    )
    .unwrap();
    tx.rollback().unwrap();
    assert_eq!(query_from(&connection, query()).unwrap().items.len(), 4);
    connection
        .execute(
            "update note_pages set status='archived' where id='page-00000'",
            [],
        )
        .unwrap();
    assert_eq!(
        query_from(
            &connection,
            PageCatalogQuery {
                view: "archive".to_owned(),
                ..query()
            }
        )
        .unwrap()
        .items[0]
            .title,
        "笔记 0"
    );
    connection
        .execute("delete from note_pages where id=?1", [&dependent])
        .unwrap();
    assert!(
        query_from(
            &connection,
            PageCatalogQuery {
                category: "dependent".to_owned(),
                ..query()
            }
        )
        .unwrap()
        .items
        .is_empty()
    );
}

#[test]
fn catalog_cursor_is_bound_to_scope_and_committed_generation() {
    let connection = crate::notes::tests::migrated_connection();
    seed(&connection, 25);
    let first = query_from(&connection, query()).unwrap();
    let cursor = first.next_cursor.unwrap();
    assert!(
        query_from(
            &connection,
            PageCatalogQuery {
                cursor: Some(cursor.clone()),
                category: "dependent".to_owned(),
                ..query()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("note_catalog_cursor_scope")
    );
    connection
        .execute(
            "update note_pages set title='renamed' where id='page-00000'",
            [],
        )
        .unwrap();
    assert!(
        query_from(
            &connection,
            PageCatalogQuery {
                cursor: Some(cursor),
                ..query()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("note_catalog_changed")
    );
    assert!(
        query_from(
            &connection,
            PageCatalogQuery {
                cursor: Some("[]".to_owned()),
                ..query()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("note_catalog_cursor_invalid")
    );
    assert!(
        query_from(
            &connection,
            PageCatalogQuery {
                limit: Some(201),
                ..query()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("note_catalog_query_invalid")
    );
}

#[test]
fn catalog_tied_timestamp_seek_does_not_rescan_prior_pages() {
    let connection = crate::notes::tests::migrated_connection();
    seed(&connection, 2400);
    connection
        .execute(
            "update note_pages set updated_at='2026-10-23T00:00:00Z'",
            [],
        )
        .unwrap();
    let sql = query_sql(false, false);
    let mut statement = connection.prepare(&sql).unwrap();
    let rows = statement
        .query_map(
            rusqlite::params![
                "active",
                order_keys::utc_millis("2026-10-23T00:00:00Z").unwrap(),
                "page-02300",
                21
            ],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 21);
    assert_eq!(rows[0], "page-02301");
    assert_eq!(statement.get_status(StatementStatus::FullscanStep), 0);
    assert!(statement.get_status(StatementStatus::VmStep) < 1000);
}

#[test]
fn catalog_wire_targets_and_conflicts_keep_identity_and_current_generation() {
    let mut connection = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&connection);
    let id = reading(&mut connection, "highlight", "quote");
    seed(&connection, 25);
    connection
        .execute(
            "update note_pages set updated_at='2026-10-23T23:59:59Z' where id=?1",
            [&id],
        )
        .unwrap();
    let first = query_from(&connection, query()).unwrap();
    let json = serde_json::to_value(&first).unwrap();
    let item = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == format!("page:{id}"))
        .unwrap();
    assert_eq!(item["target"]["kind"], "reading");
    assert_eq!(item["target"]["pageId"], id);
    assert_eq!(item["target"]["noteId"], item["owner"]["id"]);
    assert!(item.get("document").is_none());
    connection
        .execute(
            "update note_pages set revision=revision+1 where id=?1",
            [&id],
        )
        .unwrap();
    let error = NotesCommandError::from(
        query_from(
            &connection,
            PageCatalogQuery {
                cursor: first.next_cursor,
                ..query()
            },
        )
        .unwrap_err(),
    );
    let error = serde_json::to_value(error).unwrap();
    assert_eq!(error["code"], "revision_conflict");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("note_catalog_changed")
    );
    assert!(error["currentRevision"].as_i64().unwrap() > first.revision);
    assert!(
        serde_json::from_value::<PageCatalogQuery>(
            serde_json::json!({"category":"all","view":"recent","documents":"unexpected"})
        )
        .is_err()
    );
}
