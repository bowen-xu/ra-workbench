use super::{LifecycleFailure, LifecycleResources, Request, apply};
use crate::{
    data_interface::Database,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_duplicates::tests::fixture::{Fixture as CoreFixture, PAGE},
    },
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub resources: LifecycleResources,
    database_path: PathBuf,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let data = CoreFixture::new();
        let database_path = {
            let db = data.db.lock().unwrap();
            PathBuf::from(db.path().expect("the existing fixture owns a SQL/log pair"))
        };
        {
            let db = data.db.lock().unwrap();
            db.execute_batch(
                "insert into research_projects(id,title,summary,created_at)
                 values('p-old','Original','','2026'),('p-a','A','','2026'),('p-b','B','','2026');
                 insert into project_items(project_id,item_id,added_at) values('p-old','survivor','2026');
                 insert into collections(id,name,description) values('group','Group','');
                 insert into collection_memberships(item_id,collection_id)
                 values('survivor','group'),('source','group');
                 insert into identifiers(id,item_id,kind,normalized_value,display_value)
                 values('identifier','survivor','doi','synthetic-doi','Synthetic DOI');
                 insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
                 values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'synthetic.pdf','application/pdf',1,'2026');
                 insert into attachments(id,item_id,content_sha256,display_name,created_at)
                 values('attachment','survivor',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','Synthetic.pdf','2026');
                 create table synthetic_lifecycle_effects(seq integer primary key,value text not null);",
            )
            .unwrap();
        }
        let package = tempfile::tempdir().unwrap();
        let directory = package.path().join("literature_library");
        fs::create_dir_all(directory.join("data")).unwrap();
        fs::write(
            directory.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(directory.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for suffix in [
            "archive",
            "restore",
            "trash",
            "restore-trash",
            "project-delete",
            "project-insert",
            "affected-policy",
        ] {
            let name = format!("literature-lifecycle-{suffix}.v1.sql");
            fs::copy(source.join(&name), directory.join("data").join(name)).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(package.path().into()));
        let selected = catalog.discover().unwrap().packages.remove(0);
        let resources = LifecycleResources::load(&catalog, &selected).unwrap();
        Self {
            data,
            resources,
            database_path,
            _package: package,
        }
    }

    pub fn apply(&self, request: &Request, ids: &[String]) -> Result<(), LifecycleFailure> {
        apply(
            &mut self.data.db.lock().unwrap(),
            &self.resources,
            request,
            ids,
        )
    }

    pub fn flags(&self, id: &str) -> (Option<String>, Option<String>) {
        self.data
            .db
            .lock()
            .unwrap()
            .query_row(
                "select archived_at,trashed_at from bibliographic_items where id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    pub fn effects(&self) -> Vec<String> {
        rows(
            &self.data.db.lock().unwrap(),
            "select value from synthetic_lifecycle_effects order by seq",
        )
    }

    pub fn relations(&self) -> Value {
        let db = self.data.db.lock().unwrap();
        json!({
            "memberships":rows(&db,"select json_array(item_id,collection_id) from collection_memberships order by item_id,collection_id"),
            "identifiers":rows(&db,"select json_array(id,item_id,kind,normalized_value,display_value) from identifiers order by id"),
            "attachments":rows(&db,"select json_array(id,item_id,content_sha256,display_name) from attachments order by id")
        })
    }

    pub fn state(&self) -> Value {
        let db = self.data.db.lock().unwrap();
        let mut value = json!({
            "papers":rows(&db,"select json_array(id,title,archived_at,trashed_at,updated_at) from bibliographic_items order by id"),
            "projects":rows(&db,"select json_array(project_id,item_id,added_at) from project_items order by project_id,item_id"),
            "effects":rows(&db,"select json_array(seq,value) from synthetic_lifecycle_effects order by seq"),
            "noteCatalog":rows(&db,"select json_array(id,title,category,source_id,source_label,status,revision) from note_page_catalog order by id"),
            "noteMetadata":rows(&db,"select json_array(page_id,search_text,folded_text) from note_catalog_metadata_text order by page_id")
        });
        drop(db);
        value["relations"] = self.relations();
        value["note"] = self.data.note();
        assert_eq!(value["note"]["pageId"], PAGE);
        value
    }

    pub fn reopen(self) -> Self {
        Self {
            data: self.data.reopen(),
            ..self
        }
    }

    pub fn reopen_fenced(mut self) -> Self {
        assert!(self.data.db.lock().is_err());
        assert_eq!(Arc::strong_count(&self.data.db), 1);
        let old_owner = self.data.db;
        drop(old_owner);
        // The partially moved Core fixture still retains its original TempDir.
        let mut sql = Connection::open(&self.database_path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        self.data.db =
            Database::open(sql, &self.database_path.parent().unwrap().join("data-log")).unwrap();
        self
    }
}

pub(super) fn rows(db: &Connection, sql: &str) -> Vec<String> {
    db.prepare(sql)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
