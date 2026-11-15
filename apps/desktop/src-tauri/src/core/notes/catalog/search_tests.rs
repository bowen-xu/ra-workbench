use super::model::PageCatalogSort;
use super::*;
use crate::notes::{
    document_repository::{load_document_from, save_document_from},
    model::NoteDocumentSaveRequest,
};
use serde_json::json;

fn set_text(db: &mut Connection, id: &str, text: &str) {
    let mut snapshot = load_document_from(db, id).unwrap();
    snapshot.document.blocks.truncate(1);
    snapshot.document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]});
    save_document_from(
        db,
        NoteDocumentSaveRequest {
            document: snapshot.document,
            expected_revision: snapshot.page.revision,
            save_reason: "manual".to_owned(),
        },
    )
    .unwrap();
}
fn search(db: &Connection, text: &str) -> PageCatalogPage {
    query_from(
        db,
        PageCatalogQuery {
            search: Some(text.to_owned()),
            ..super::tests::query()
        },
    )
    .unwrap()
}

#[test]
fn catalog_revision_is_the_same_for_browsing_sorting_and_search() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    set_text(&mut db, &id, "native-needle-1737");
    let found = search(&db, "native-needle-1737");
    assert_eq!(found.items.len(), 1);
    let checkpoint = query_from(
        &db,
        PageCatalogQuery {
            limit: Some(1),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(
        found.revision, checkpoint.revision,
        "a metadata checkpoint must validate the same source generation as search"
    );
    let sorted = query_from(
        &db,
        PageCatalogQuery {
            sort: Some(PageCatalogSort {
                field: "title".to_owned(),
                direction: "ascending".to_owned(),
            }),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(sorted.revision, found.revision);
    set_text(&mut db, &id, "native-needle-1737 updated");
    let updated = search(&db, "native-needle-1737");
    assert!(updated.revision > found.revision);
    assert_eq!(
        query_from(&db, super::tests::query()).unwrap().revision,
        updated.revision
    );
}

#[test]
fn persistent_search_handles_unicode_short_terms_cross_block_terms_and_literal_punctuation() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    set_text(
        &mut db,
        &id,
        "ABCD核心概念\nİSTANBUL ΟΣ \"quoted\" 100% literal* plain_name",
    );
    for text in [
        "abcd",
        "BCD",
        "核心",
        "念",
        "核心概念",
        "İSTANBUL",
        "ος",
        "\"quoted\"",
        "100%",
        "literal*",
        "plain_name",
        "ABCD quoted",
        "概念 İSTANBUL",
    ] {
        let result = search(&db, text);
        assert_eq!(result.items.len(), 1, "{text}");
        assert_eq!(result.items[0].id, format!("page:{id}"));
        assert!(result.items[0].snippet.contains("ABCD"));
    }
    for text in [
        "ABCD missing",
        "literal?",
        "quoted OR missing",
        "column:ABCD",
        "σabc",
        "iSTANBUL",
    ] {
        assert!(search(&db, text).items.is_empty(), "{text}");
    }
    // An indexed query must not evaluate projection functions or open documents.
    db.create_scalar_function(
        "ra_note_block_projection_v1",
        3,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        |_| -> rusqlite::Result<String> { Err(rusqlite::Error::InvalidQuery) },
    )
    .unwrap();
    assert_eq!(search(&db, "ABCD").items.len(), 1);
    db.execute(
        "update note_pages set is_favorite=1,revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(search(&db, "ABCD").items.len(), 1);
    assert_eq!(
        query_from(&db, super::tests::query()).unwrap().items.len(),
        1
    );
}

#[test]
fn quote_only_changes_refresh_search_even_when_the_cached_header_is_unchanged() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "highlight", "oldquote123");
    assert_eq!(search(&db, "oldquote123").items.len(), 1);
    let title = search(&db, "oldquote123").items[0].title.clone();
    db.execute(
        "update reading_note_sources set quote_exact='newquote456' where page_id=?1",
        [&id],
    )
    .unwrap();
    assert!(search(&db, "oldquote123").items.is_empty());
    let result = search(&db, "newquote456");
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].title, title);
    let revision = result.revision;
    db.execute(
        "update reading_note_sources set quote_exact='NEWQUOTE456' where page_id=?1",
        [&id],
    )
    .unwrap();
    assert!(
        search(&db, "newquote456").revision > revision,
        "a case-only quote edit must invalidate search cursors without reindexing identical tokens"
    );
}

#[test]
fn search_is_global_across_browse_filters_and_obeys_trash_and_saved_transaction_state() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "highlight", "a quote");
    set_text(&mut db, &id, "searchable");
    let result = query_from(
        &db,
        PageCatalogQuery {
            category: "independent".to_owned(),
            view: "favorites".to_owned(),
            search: Some("searchable".to_owned()),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(result.items.len(), 1);
    db.execute(
        "update note_pages set status='archived',revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(search(&db, "searchable").items.len(), 1);
    db.execute(
        "update note_pages set status='trashed',revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    assert!(search(&db, "searchable").items.is_empty());
    assert_eq!(
        query_from(
            &db,
            PageCatalogQuery {
                view: "trash".to_owned(),
                search: Some("searchable".to_owned()),
                ..super::tests::query()
            }
        )
        .unwrap()
        .items
        .len(),
        1
    );
    let tx = db.transaction().unwrap();
    tx.execute(
        "update note_blocks set content_json=?2 where page_id=?1",
        rusqlite::params![
            id,
            json!({"kind":"inline","items":[{"type":"text","text":"replacement","styles":[]}]})
                .to_string()
        ],
    )
    .unwrap();
    tx.rollback().unwrap();
    assert!(
        query_from(
            &db,
            PageCatalogQuery {
                view: "trash".to_owned(),
                search: Some("replacement".to_owned()),
                ..super::tests::query()
            }
        )
        .unwrap()
        .items
        .is_empty()
    );
}

#[test]
fn search_paginates_distinct_pages_and_rejects_changed_generation_or_query_scope() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    for _ in 0..22 {
        let id = super::tests::reading(&mut db, "page_note", "");
        set_text(&mut db, &id, "shared-term shared-term");
    }
    let first = search(&db, "shared-term");
    assert_eq!(first.items.len(), 20);
    let cursor = first.next_cursor.unwrap();
    let next = query_from(
        &db,
        PageCatalogQuery {
            search: Some("shared-term".to_owned()),
            cursor: Some(cursor.clone()),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(next.items.len(), 2);
    assert!(
        next.items
            .iter()
            .all(|item| !first.items.iter().any(|before| before.id == item.id))
    );
    assert!(
        query_from(
            &db,
            PageCatalogQuery {
                search: Some("other".to_owned()),
                cursor: Some(cursor.clone()),
                ..super::tests::query()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("note_catalog_cursor_scope")
    );
    db.execute(
        "update reading_note_sources set quote_exact='changed without a title change'",
        [],
    )
    .unwrap();
    assert!(matches!(
        query_from(
            &db,
            PageCatalogQuery {
                search: Some("shared-term".to_owned()),
                cursor: Some(cursor),
                ..super::tests::query()
            }
        ),
        Err(NotesError::CatalogChanged { .. })
    ));
    let cursor = search(&db, "shared-term").next_cursor.unwrap();
    db.execute("update note_pages set is_favorite=1", [])
        .unwrap();
    assert!(matches!(
        query_from(
            &db,
            PageCatalogQuery {
                search: Some("shared-term".to_owned()),
                cursor: Some(cursor),
                ..super::tests::query()
            }
        ),
        Err(NotesError::CatalogChanged { .. })
    ));
}

#[test]
fn invalid_index_rows_fail_search_with_a_specific_cause_instead_of_false_empty_results() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    db.execute(
        "update note_blocks set content_json='{\"kind\":\"table\"}' where page_id=?1",
        [&id],
    )
    .unwrap();
    let failure = query_from(
        &db,
        PageCatalogQuery {
            search: Some("anything".to_owned()),
            ..super::tests::query()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(failure.contains("note_catalog_invalid_text"));
    assert!(!failure.contains(&id));
    assert_eq!(
        query_from(&db, super::tests::query()).unwrap().items.len(),
        1
    );
}

#[test]
fn search_sort_reuses_persisted_keys_and_binds_its_cursor_to_direction() {
    let db = crate::notes::tests::migrated_connection();
    for (id, title) in [("a", "命中2"), ("b", "命中10"), ("c", "命中02")] {
        db.execute("insert into note_pages(id,title,created_at,updated_at)values(?1,?2,'2026-10-23T00:00:00Z','2026-10-23T00:00:00Z')",rusqlite::params![id,title]).unwrap();
    }
    let mut request = PageCatalogQuery {
        search: Some("命中".to_owned()),
        sort: Some(model::PageCatalogSort {
            field: "title".to_owned(),
            direction: "descending".to_owned(),
        }),
        limit: Some(1),
        ..super::tests::query()
    };
    let first = query_from(&db, request.clone()).unwrap();
    assert_eq!(first.items[0].id, "page:b");
    request.cursor = first.next_cursor;
    let second = query_from(&db, request.clone()).unwrap();
    assert_eq!(second.items[0].id, "page:a");
    request.cursor = second.next_cursor;
    assert_eq!(
        query_from(&db, request.clone()).unwrap().items[0].id,
        "page:c"
    );
    request.sort.as_mut().unwrap().direction = "ascending".to_owned();
    assert!(
        query_from(&db, request)
            .unwrap_err()
            .to_string()
            .contains("note_catalog_cursor_scope")
    );
}

#[test]
fn nul_in_literal_search_uses_saved_projection_without_truncating_the_query() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    set_text(&mut db, &id, "prefix abc\0def suffix");
    assert_eq!(search(&db, "abc\0def").items.len(), 1);
    assert!(search(&db, "abc\0wrong").items.is_empty());
}
