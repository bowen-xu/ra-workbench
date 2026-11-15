use super::*;

#[test]
fn object_staging_upgrade_validates_identifiers_and_retains_unowned_intents() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 80);
    crate::storage::migrate(&mut db).unwrap();
    let hash = "a".repeat(64);
    db.execute(
        "insert into agent_object_staging(sha256,byte_size) values(?1,0)",
        [&hash],
    )
    .unwrap();
    for invalid in ["short".to_owned(), "A".repeat(64), "g".repeat(64)] {
        assert!(
            db.execute(
                "insert into agent_object_staging(sha256,byte_size) values(?1,0)",
                [invalid]
            )
            .is_err()
        );
    }
    for size in [-1_i64, 268435457] {
        assert!(
            db.execute("update agent_object_staging set byte_size=?1", [size])
                .is_err()
        );
    }
    assert_eq!(
        db.query_row("select byte_size from agent_object_staging", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let violations: i64 = db
        .query_row("select count(*) from pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}
