use super::*;
use crate::data_interface::{Database, SharedDatabase};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

const FILES: &[&str] = &[
    "list-active-ids",
    "search-fts-ids",
    "search-year-ids",
    "item-main",
    "item-creators",
    "item-tags",
    "item-attachments",
    "item-project-ids",
    "item-metadata-sources",
    "list-trash-ids",
    "search-trash-fts-ids",
    "search-trash-year-ids",
];

pub(super) struct Fixture {
    _directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: CatalogResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(directory.path().join("catalog.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &directory.path().join("data-log")).unwrap();
        let (catalog, package) = package(directory.path());
        let resources = CatalogResources::load(&catalog, &package).unwrap();
        assert_eq!(resources.package_key(), package.package_key);
        Self {
            _directory: directory,
            db,
            resources,
        }
    }

    pub fn trash_resources(&self) -> CatalogResources {
        let (catalog, package) = package(self._directory.path());
        CatalogResources::load_trash(&catalog, &package).unwrap()
    }

    pub fn item(&self, id: &str, title: &str, year: Option<i64>, at: &str) {
        insert_item(&self.db.lock().unwrap(), id, title, year, at);
    }

    pub fn related(&self, id: &str) {
        let db = self.db.lock().unwrap();
        db.execute_batch("insert into creators(id,family_name,given_name,literal_name)
            values('creator-z','Zed','Z',''),('creator-a','Alpha','A','');
            insert into tags(id,name) values('tag-z','zeta'),('tag-a','Alpha');
            insert into research_projects(id,title,summary,created_at)
            values('project-later','Later','','now'),('project-first','First','','now');
            insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
            values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','objects/a','application/pdf',17,'now');").unwrap();
        for (creator, position) in [("creator-z", 1), ("creator-a", 0)] {
            db.execute("insert into item_creators(item_id,creator_id,role,position) values(?1,?2,'author',?3)", params![id, creator, position]).unwrap();
        }
        for tag in ["tag-z", "tag-a"] {
            db.execute(
                "insert into item_tags(item_id,tag_id) values(?1,?2)",
                params![id, tag],
            )
            .unwrap();
        }
        for (project, at) in [
            ("project-later", "2025-01-02"),
            ("project-first", "2025-01-01"),
        ] {
            db.execute(
                "insert into project_items(item_id,project_id,added_at) values(?1,?2,?3)",
                params![id, project, at],
            )
            .unwrap();
        }
        for (attachment, at) in [
            ("attachment-later", "2025-01-02"),
            ("attachment-first", "2025-01-01"),
        ] {
            db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at)
                values(?1,?2,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',?1,?3)", params![attachment,id,at]).unwrap();
        }
        db.execute(
            "insert into identifiers(id,item_id,kind,normalized_value,display_value)
            values('doi',?1,'doi','10.1000/test','10.1000/TEST')",
            [id],
        )
        .unwrap();
        for field in ["title", "abstractText"] {
            db.execute("insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at)
                values(?1,?2,'manual',1,'now')", params![id,field]).unwrap();
        }
    }
}

pub(super) fn package(directory: &Path) -> (RuntimePluginCatalog, RuntimePluginPackage) {
    let root = directory.join("packages/literature");
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("index.js"), "export {};").unwrap();
    fs::write(
        root.join("manifest.json"),
        r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
    )
    .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in FILES {
        let name = format!("catalog-{name}.v1.sql");
        fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(directory.join("packages")));
    let package = catalog.discover().unwrap().packages.remove(0);
    (catalog, package)
}

pub(super) fn insert_item(db: &Connection, id: &str, title: &str, year: Option<i64>, at: &str) {
    db.execute(
        "insert into bibliographic_items(id,item_type,title,issued_year,created_at,updated_at)
        values(?1,'journalArticle',?2,?3,?4,?4)",
        params![id, title, year, at],
    )
    .unwrap();
}

pub(super) fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}

pub(super) fn texts(rows: &DataRows) -> Vec<String> {
    rows.rows
        .iter()
        .map(|row| match &row[0] {
            DataValue::Text(value) => value.clone(),
            _ => panic!("expected original text ID"),
        })
        .collect()
}

pub(super) fn observe_reads(db: &Connection) -> Arc<Mutex<Vec<String>>> {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&reads);
    db.authorizer(Some(move |context: AuthContext<'_>| {
        match context.action {
            AuthAction::Read { table_name, .. } => observed.lock().unwrap().push(table_name.into()),
            AuthAction::Transaction { .. } => observed.lock().unwrap().push("TRANSACTION".into()),
            _ => {}
        }
        Authorization::Allow
    }));
    reads
}

pub(super) fn stop_observing(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}
