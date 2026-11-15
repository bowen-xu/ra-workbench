use super::*;
use rusqlite::StatementStatus;

#[test]
fn rare_full_text_query_uses_fts_and_reads_no_canonical_bodies() {
    let db = crate::notes::tests::migrated_connection();
    let tx = db.unchecked_transaction().unwrap();
    for index in 0..2400 {
        tx.execute("insert into note_pages(id,title,created_at,updated_at)values(?1,'metadata','2026-10-23T00:00:00Z','2026-10-23T00:00:00Z')",[format!("page-{index:04}")]).unwrap();
        let text = if index == 1234 {
            "rareabcdef __"
        } else {
            "unrelated projection"
        };
        tx.execute("insert into note_blocks(id,scope_kind,page_id,block_type,props_json,content_json,plain_text,created_at,updated_at)values(?1,'page',?2,'paragraph','{}',?3,'legacy','now','now')",rusqlite::params![format!("block-{index:04}"),format!("page-{index:04}"),serde_json::json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}).to_string()]).unwrap();
    }
    tx.commit().unwrap();
    let query = crate::notes::catalog::tests::query();
    for text in ["rareabcdef", "__"] {
        let terms = terms(text).unwrap();
        let sql = search_sql(&terms, None);
        assert!(!sql.contains("note_blocks"));
        assert!(!sql.contains("content_json"));
        let mut statement = db.prepare(&sql).unwrap();
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(parameters(&query, &terms, None, 20)),
                |_| Ok(()),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let count = rows.len();
        assert_eq!(count, 1);
        let scans = statement.get_status(StatementStatus::FullscanStep);
        let steps = statement.get_status(StatementStatus::VmStep);
        eprintln!(
            "note_catalog_search_measurement term_length={} returned_rows={count} fullscan_steps={scans} vm_steps={steps}",
            text.chars().count()
        );
        if text.len() > 2 {
            assert!(
                scans < 64,
                "FTS must not scan all 2400 cached bodies: {scans}"
            );
            assert!(
                steps < 2000,
                "rare FTS query should stay selective: {steps}"
            );
            let mut explain = db.prepare(&format!("explain query plan {sql}")).unwrap();
            let plan = explain
                .query_map(
                    rusqlite::params_from_iter(parameters(&query, &terms, None, 20)),
                    |row| row.get::<_, String>(3),
                )
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains("VIRTUAL TABLE INDEX"), "{plan}");
        } else {
            assert!(
                scans >= 2400,
                "document the short-term projection scan rather than claiming an FTS seek"
            );
        }
    }
}

#[test]
fn search_terms_follow_javascript_whitespace_and_validate_bounds() {
    assert_eq!(
        terms("\u{feff}核心\u{a0}概念\u{2029}ALPHA\u{feff}").unwrap(),
        vec!["核心", "概念", "alpha"]
    );
    assert_eq!(terms("\u{0085}").unwrap(), vec!["\u{0085}"]);
    assert!(terms(&"x".repeat(513)).is_err());
    assert!(terms(&vec!["term"; 1400].join(" ")).is_err());
    assert!(
        terms(
            &(0..33)
                .map(|index| format!("term{index}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .is_err()
    );
}
