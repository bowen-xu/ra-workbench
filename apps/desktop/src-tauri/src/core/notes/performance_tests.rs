use std::time::{Duration, Instant};

use rusqlite::{Connection, params};

use crate::storage::migrate;

use super::{
    model::NoteSearchQuery,
    search::{rebuild_note_index_from, search_notes_from},
};

#[test]
fn dense_library_rebuild_and_search_stay_within_p0_budgets() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate(&mut connection).expect("migrate");
    let transaction = connection.transaction().expect("fixture transaction");
    for index in 0..4_000 {
        let id = format!("performance-page-{index:04}");
        let title = if index == 3_999 {
            "needle page".to_owned()
        } else {
            format!("Page {index}")
        };
        transaction.execute(
            "insert into note_pages
             (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values (?1, ?2, ?3, 'active', 0, 0, 1, 'now', 'now')",
            params![id, title, index],
        ).expect("page");
        transaction
            .execute(
                "insert into note_blocks
             (id, scope_kind, page_id, position, block_type, props_json, content_json, plain_text,
              schema_version, revision, status, is_favorite, created_at, updated_at)
             values (?1, 'page', ?2, 0, 'paragraph', '{}', '{\"kind\":\"inline\",\"items\":[]}',
                     ?3, 1, 0, 'active', 0, 'now', 'now')",
                params![
                    format!("performance-block-{index:04}"),
                    id,
                    format!("fixed fixture text {index}")
                ],
            )
            .expect("block");
    }
    transaction.commit().expect("fixture commit");

    let rebuild_started_at = Instant::now();
    rebuild_note_index_from(&mut connection).expect("rebuild");
    let rebuild = rebuild_started_at.elapsed();
    let query = || NoteSearchQuery {
        search: Some("needle".to_owned()),
        status: Some("active".to_owned()),
        object_kinds: Some(vec!["note_page".to_owned()]),
        project_id: None,
        tag_id: None,
        paper_id: None,
        note_type: None,
        source_state: None,
        is_favorite: None,
        limit: Some(20),
        cursor: None,
    };
    let mut samples = Vec::new();
    for _ in 0..20 {
        let started_at = Instant::now();
        let result = search_notes_from(&connection, query()).expect("search");
        samples.push(started_at.elapsed());
        assert_eq!(result.items.len(), 1);
    }
    samples.sort();
    let p50 = samples[10];
    let p95 = samples[19];
    eprintln!("notes performance: rebuild={rebuild:?}, search_p50={p50:?}, search_p95={p95:?}");
    assert!(rebuild < Duration::from_secs(5));
    assert!(p95 < Duration::from_millis(250));
}
