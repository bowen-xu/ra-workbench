use super::*;
use crate::notes::document_repository::load_document_from;

#[test]
fn order_projection_upgrades_197_198_199_and_reopens_without_rewriting_documents() {
    for deployed in [197, 198, 199] {
        let path = std::env::temp_dir().join(format!(
            "note-order-upgrade-{deployed}-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 197);
        crate::notes::tests::insert_paper_fixture(&db);
        let id = super::tests::reading(&mut db, "highlight", "原始摘录");
        let before = serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap();
        for &(version, sql) in crate::storage_migrations::MIGRATIONS {
            if version > 197 && version <= deployed {
                let tx = db.transaction().unwrap();
                tx.execute_batch(sql).unwrap();
                tx.execute(
                    "insert into schema_migrations(version,applied_at)values(?1,'now')",
                    [version],
                )
                .unwrap();
                tx.commit().unwrap();
            }
        }
        if deployed >= 198 {
            // Retain evidence of both deployed trigger defects, rather than
            // silently rewriting already-run SQL to make a fresh DB pass.
            let error=db.execute("insert into note_pages(id,title,created_at,updated_at)values('failed','never committed','2026-10-23T00:00:00Z','2026-10-23T00:00:00Z')",[]).unwrap_err();
            assert!(
                error.to_string().contains(if deployed == 198 {
                    "17 columns but 10 values"
                } else {
                    "note_catalog_metadata_text.page_id"
                }),
                "{error}"
            );
        }
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(
            serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap(),
            before
        );
        assert!(
            !db.query_row(
                "select exists(select 1 from note_pages where id='failed')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
        );
        db.execute("insert into note_pages(id,title,created_at,updated_at)values('new','新增2','2026-10-23T00:00:00Z','2026-10-23T00:00:00Z')",[]).unwrap();
        db.execute(
            "update note_pages set title='新增10',is_favorite=1,revision=revision+1 where id='new'",
            [],
        )
        .unwrap();
        let mut request = super::tests::query();
        request.search = Some("新增10".to_owned());
        assert_eq!(query_from(&db, request).unwrap().items[0].id, "page:new");
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(
            serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            db.query_row("pragma integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(
            !db.prepare("pragma foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
