use super::*;

#[test]
fn upgrades_v169_optional_explanation_and_keeps_deleted_identity_and_annotations_content() {
    let path =
        std::env::temp_dir().join(format!("concept-lifecycle-{}.sqlite", uuid::Uuid::new_v4()));
    {
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("pragma foreign_keys=on;create table schema_migrations(version integer primary key,applied_at text not null);").unwrap();
        for &(version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 170) {
            apply_migration(&mut db, version, sql).unwrap();
        }
        db.execute("insert into concepts(id,label,explanation,created_at,updated_at) values('a','领域','原解释','now','now')",[]).unwrap();
        db.execute("insert into concepts(id,label,explanation,created_at,updated_at) values('b','关系','原解释','now','now')",[]).unwrap();
        db.execute("insert into note_pages(id,title,status,position,created_at,updated_at) values('note','保留笔记','active',0,'now','now')",[]).unwrap();
        db.execute("insert into nal_statements(id,subject,predicate,origin) values('s','{\"kind\":\"product\",\"terms\":[{\"kind\":\"concept\",\"id\":\"a\"},{\"kind\":\"concept\",\"id\":\"b\"}]}','{\"kind\":\"concept\",\"id\":\"b\"}','human')",[]).unwrap();
        db.execute(
            "insert into nal_annotations(id,statement_id,note_page_id) values('ann','s','note')",
            [],
        )
        .unwrap();
        migrate(&mut db).unwrap();
        assert_eq!(
            db.query_row("select explanation from concepts where id='a'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "原解释"
        );
        assert_eq!(
            db.query_row("select count(*) from nal_annotations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute("update concepts set explanation='' where id='a'", [])
            .unwrap();
        db.execute("update concepts set deleted_at='now' where id='a'", [])
            .unwrap();
        assert_eq!(
            db.query_row("select count(*) from nal_statements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("select count(*) from nal_annotations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("select count(*) from note_pages where id='note'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute("insert into concepts(id,label,explanation,created_at,updated_at) values('replacement','领域','','now','now')",[]).unwrap();
        assert!(
            db.execute("update concepts set deleted_at=null where id='a'", [])
                .is_err()
        );
        assert!(
            db.query_row("pragma foreign_keys", [], |r| r.get::<_, bool>(0))
                .unwrap()
        );
    }
    {
        let mut db = Connection::open(&path).unwrap();
        migrate(&mut db).unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from concepts where deleted_at is null",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "select count(*) from concepts where id='a' and deleted_at is not null",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    std::fs::remove_file(path).unwrap();
}
