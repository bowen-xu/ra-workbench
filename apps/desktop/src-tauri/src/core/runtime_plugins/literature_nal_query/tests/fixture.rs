use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::RuntimePluginCatalog,
};
use research_workbench_native_sdk::nal_query::SavePlan;
use rusqlite::Connection;
use std::{fs, path::Path};

pub(super) struct Fixture {
    pub directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: NalQueryResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut connection = Connection::open(directory.path().join("query.sqlite3")).unwrap();
        crate::storage::migrate(&mut connection).unwrap();
        let db = Database::open(connection, &directory.path().join("data-log")).unwrap();
        let root = directory.path().join("packages/literature");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(root.join("index.js"), "export {};").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        for file in ["nal-query-read.v1.sql", "nal-query-save.v1.sql"] {
            fs::copy(source.join(file), root.join("data").join(file)).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = NalQueryResources::load(&catalog, &package).unwrap();
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn reopen(self) -> Self {
        let Self {
            directory,
            db,
            resources,
        } = self;
        drop(db);
        let mut connection = Connection::open(directory.path().join("query.sqlite3")).unwrap();
        crate::storage::migrate(&mut connection).unwrap();
        let db = Database::open(connection, &directory.path().join("data-log")).unwrap();
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn raw(&self) -> research_workbench_native_sdk::nal_query::StoredRow {
        self.resources.read_raw(&self.db.lock().unwrap()).unwrap()
    }
}

pub(super) fn plan() -> SavePlan {
    SavePlan {
        definitions_json: "[]".into(),
        cache_json: Some(r#"{"synthetic":1}"#.into()),
        legacy_imported: true,
    }
}
