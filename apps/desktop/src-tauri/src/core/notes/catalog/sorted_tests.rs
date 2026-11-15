use super::{
    model::{PageCatalogQuery, PageCatalogSort},
    sorted::{ordering, query_sql},
    tests::query,
};
use rusqlite::{Connection, StatementStatus, params, types::Value};

fn request(field: &str, direction: &str) -> PageCatalogQuery {
    PageCatalogQuery {
        sort: Some(PageCatalogSort {
            field: field.to_owned(),
            direction: direction.to_owned(),
        }),
        limit: Some(2),
        ..query()
    }
}
fn all(db: &Connection, mut request: PageCatalogQuery) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        let page = super::query_from(db, request.clone()).unwrap();
        ids.extend(page.items.iter().map(|item| item.id.clone()));
        if let Some(cursor) = page.next_cursor {
            request.cursor = Some(cursor);
        } else {
            return ids;
        }
    }
}
fn insert(db: &Connection, id: &str, title: &str, time: &str) {
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at)values(?1,?2,?3,?3)",
        params![id, title, time],
    )
    .unwrap();
}

#[test]
fn persisted_sort_keys_seek_in_both_directions_with_stable_ties() {
    let db = crate::notes::tests::migrated_connection();
    for (id, title, time) in [
        ("a", "Alpha 2", "2026-10-23T00:00:00Z"),
        ("b", "alpha 02", "2026-10-23T00:00:00.000Z"),
        ("c", "Alpha 10", "2026-10-23T08:00:00.123+08:00"),
        ("d", "笔记2", "2026-10-23T00:00:00.124Z"),
        ("e", "笔记10", "2026-10-22T23:59:59.999Z"),
    ] {
        insert(&db, id, title, time);
    }
    let names = |ids: &[&str]| {
        ids.iter()
            .map(|id| format!("page:{id}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        all(&db, request("title", "ascending")),
        names(&["d", "e", "a", "b", "c"])
    );
    assert_eq!(
        all(&db, request("title", "descending")),
        names(&["c", "a", "b", "e", "d"])
    );
    assert_eq!(
        all(&db, request("updatedAt", "ascending")),
        names(&["e", "a", "b", "c", "d"])
    );
    assert_eq!(
        all(&db, request("updatedAt", "descending")),
        names(&["d", "c", "a", "b", "e"])
    );
    // Both fields have equal keys here. IDs always ascend, even with a descending sort.
    for field in ["category", "source"] {
        for direction in ["ascending", "descending"] {
            assert_eq!(
                all(&db, request(field, direction)),
                names(&["a", "b", "c", "d", "e"])
            );
        }
    }
    let mut favorites = request("title", "descending");
    favorites.view = "favorites".to_owned();
    db.execute(
        "update note_pages set is_favorite=1 where id in ('a','e')",
        [],
    )
    .unwrap();
    assert_eq!(all(&db, favorites), names(&["a", "e"]));
    db.execute("update note_pages set status='archived' where id='e'", [])
        .unwrap();
    let mut archive = request("title", "ascending");
    archive.view = "archive".to_owned();
    assert_eq!(all(&db, archive), names(&["e"]));
}

#[test]
fn sort_cursor_is_bound_to_sort_scope_and_generation() {
    let db = crate::notes::tests::migrated_connection();
    for index in 0..4 {
        insert(
            &db,
            &format!("p{index}"),
            &format!("笔记{index}"),
            "2026-10-23T00:00:00Z",
        );
    }
    let mut next = request("title", "ascending");
    next.cursor = super::query_from(&db, next.clone()).unwrap().next_cursor;
    assert!(next.cursor.is_some());
    let mut other = next.clone();
    other.sort.as_mut().unwrap().direction = "descending".to_owned();
    assert!(
        super::query_from(&db, other)
            .unwrap_err()
            .to_string()
            .contains("note_catalog_cursor_scope")
    );
    db.execute("update note_pages set title='changed' where id='p0'", [])
        .unwrap();
    assert!(
        super::query_from(&db, next)
            .unwrap_err()
            .to_string()
            .contains("note_catalog_changed")
    );
    let mut invalid = request("title", "ascending");
    invalid.sort.as_mut().unwrap().field = "title);drop table note_pages;--".to_owned();
    assert!(
        super::query_from(&db, invalid)
            .unwrap_err()
            .to_string()
            .contains("note_catalog_sort_invalid")
    );
}

#[test]
fn every_browse_sort_uses_bounded_index_ranges_including_deep_pages() {
    let db = crate::notes::tests::migrated_connection();
    let tx = db.unchecked_transaction().unwrap();
    for index in 0..2400 {
        insert(
            &tx,
            &format!("p{index:04}"),
            &format!("笔记{index}"),
            "2026-10-23T00:00:00Z",
        );
    }
    // Query-plan fixture only; canonical favorite maintenance is tested separately.
    tx.execute("update note_page_catalog set is_favorite=1", [])
        .unwrap();
    tx.commit().unwrap();
    for field in ["title", "category", "source", "updatedAt"] {
        for direction in ["ascending", "descending"] {
            for scope in ["all", "independent", "favorites"] {
                for deep in [false, true] {
                    let mut q = request(field, direction);
                    q.limit = Some(20);
                    if scope == "favorites" {
                        q.view = "favorites".to_owned();
                    } else {
                        q.category = scope.to_owned();
                    }
                    let order = ordering(q.sort.as_ref().unwrap()).unwrap();
                    let after: Value = db
                        .query_row(
                            &format!(
                                "select {} from note_page_catalog where id='p2300'",
                                order.column
                            ),
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let sql = query_sql(order, &q, deep);
                    assert!(!sql.contains("note_blocks"));
                    let mut statement = db.prepare(&sql).unwrap();
                    statement.raw_bind_parameter(1, "active").unwrap();
                    statement.raw_bind_parameter(2, &q.category).unwrap();
                    statement.raw_bind_parameter(4, after).unwrap();
                    statement.raw_bind_parameter(5, "p2300").unwrap();
                    statement.raw_bind_parameter(6, 21).unwrap();
                    let mut rows = statement.raw_query();
                    let mut count = 0;
                    while rows.next().unwrap().is_some() {
                        count += 1;
                    }
                    drop(rows);
                    assert_eq!(count, 21, "{field}/{direction}/{scope}/{deep}");
                    assert_eq!(
                        statement.get_status(StatementStatus::FullscanStep),
                        0,
                        "{sql}"
                    );
                    assert!(
                        statement.get_status(StatementStatus::VmStep) < 1000,
                        "{sql}"
                    );
                    assert_eq!(statement.get_status(StatementStatus::Sort), 0, "{sql}");
                }
            }
        }
    }
}

#[test]
fn queries_and_favorite_updates_reuse_keys_without_body_or_collation_work() {
    let db = crate::notes::tests::migrated_connection();
    insert(&db, "p", "笔记2", "2026-10-23T00:00:00Z");
    let flags = rusqlite::functions::FunctionFlags::SQLITE_UTF8
        | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC
        | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS;
    for name in ["ra_note_sort_key_v1", "ra_note_block_projection_v1"] {
        db.create_scalar_function(name, 1, flags, |_| -> rusqlite::Result<String> {
            Err(rusqlite::Error::UserFunctionError(Box::new(
                std::io::Error::other("unexpected projection"),
            )))
        })
        .unwrap();
    }
    db.execute("update note_pages set is_favorite=1 where id='p'", [])
        .unwrap();
    for field in ["title", "category", "source", "updatedAt"] {
        for direction in ["ascending", "descending"] {
            assert_eq!(all(&db, request(field, direction)), ["page:p"]);
        }
    }
}

#[test]
fn invalid_timestamp_is_preserved_and_reported_only_in_the_queried_scope() {
    let db = crate::notes::tests::migrated_connection();
    insert(&db, "bad", "bad", "now");
    db.execute("update note_pages set status='trashed' where id='bad'", [])
        .unwrap();
    assert!(all(&db, request("title", "ascending")).is_empty());
    let mut q = request("title", "ascending");
    q.view = "trash".to_owned();
    let error = super::query_from(&db, q).unwrap_err().to_string();
    assert!(error.contains("note_catalog_timestamp_invalid"));
    assert!(error.contains("pageId=bad"));
    assert_eq!(
        db.query_row(
            "select updated_at from note_pages where id='bad'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "now"
    );
}
