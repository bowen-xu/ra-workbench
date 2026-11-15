use super::*;
#[test]
fn bibliography_upgrade_preserves_deployed_checksums_and_reopens() {
    let root = std::env::temp_dir().join(format!("bibliography-upgrade-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("root");
    let path = root.join("test.sqlite");
    let mut db = Connection::open(&path).expect("db");
    db.execute_batch("pragma foreign_keys=on;create table schema_migrations(version integer primary key,applied_at text not null);").expect("schema");
    for &(version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version < 172) {
        apply_migration(&mut db, version, sql).expect("deployed migration");
    }
    let old_checksums: Vec<(i64, String)> = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .expect("query")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("checksums");
    db.execute("insert into research_projects(id,title,summary,created_at) values ('retained','Retained','','now')",[]).expect("existing data");
    migrate(&mut db).expect("upgrade");
    drop(db);
    let mut db = Connection::open(path).expect("reopen");
    migrate(&mut db).expect("reopen migrate");
    for (version, hash) in old_checksums {
        let actual: String = db
            .query_row(
                "select sha256 from migration_checksums where version=?1",
                [version],
                |r| r.get(0),
            )
            .expect("recorded checksum");
        assert_eq!(actual, hash);
    }
    assert_eq!(
        db.query_row(
            "select title from research_projects where id='retained'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("preserved"),
        "Retained"
    );
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_bibliography_entries",
            [],
            |r| r.get::<_, i64>(0)
        )
        .expect("new schema"),
        0
    );
    drop(db);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn citation_key_upgrade_preserves_deployed_173_and_reopens() {
    let root = std::env::temp_dir().join(format!("citation-upgrade-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("root");
    let path = root.join("test.sqlite");
    let mut db = Connection::open(&path).expect("db");
    db.execute_batch("pragma foreign_keys=on;create table schema_migrations(version integer primary key,applied_at text not null);").expect("schema");
    for &(version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version < 174) {
        apply_migration(&mut db, version, sql).expect("deployed migration");
    }
    let checksums = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .expect("query")
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .expect("rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("checksums");
    db.execute("insert into research_projects(id,title,summary,created_at) values ('p','Project','','now')",[]).expect("project");
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values ('m','p','Manuscript','now','now')",[]).expect("manuscript");
    db.execute("insert into manuscript_bibliography_entries(id,manuscript_id,citation_key,original_key,original_bibtex,imported_draft_json,bibtex_entry) values ('entry','m','key','key','original','{}','canonical')",[]).expect("existing reference");
    migrate(&mut db).expect("upgrade");
    drop(db);
    let mut db = Connection::open(path).expect("reopen");
    migrate(&mut db).expect("reopen migration");
    for (version, hash) in checksums {
        assert_eq!(
            db.query_row(
                "select sha256 from migration_checksums where version=?1",
                [version],
                |r| r.get::<_, String>(0)
            )
            .expect("checksum"),
            hash
        );
    }
    assert_eq!(
        db.query_row(
            "select citation_key from manuscript_bibliography_entries where id='entry'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("reference"),
        "key"
    );
    db.execute(
        "insert into manuscript_citation_key_bases values ('m','entry','key')",
        [],
    )
    .expect("new key identity");
    drop(db);
    fs::remove_dir_all(root).expect("cleanup");
}
