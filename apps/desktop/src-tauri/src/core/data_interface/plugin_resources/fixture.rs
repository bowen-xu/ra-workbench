use crate::{
    data_interface::{Database, SharedDatabase},
    notes,
    runtime_plugins::{RuntimePluginCatalog, RuntimePluginPackage, data::AcceptedDataResource},
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue,
};
use rusqlite::{Connection, Transaction};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub(super) const READ_SQL: &str =
    "select id,note_page_id from nal_annotations where concept_id=?1 order by id";
pub(super) const INSERT_SQL: &str = "insert into nal_annotations \
    (id,concept_id,statement_id,note_page_id) values(?1,?2,null,?3) on conflict do nothing";

pub(super) struct DatabaseFixture {
    pub directory: tempfile::TempDir,
    pub db: SharedDatabase,
}

impl DatabaseFixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = open(directory.path());
        db.lock()
            .unwrap()
            .execute(
                "insert into concepts(id,label,explanation,created_at,updated_at) \
             values('concept','Synthetic concept','','now','now')",
                [],
            )
            .unwrap();
        Self { directory, db }
    }

    pub fn reopen(self) -> Self {
        let Self { directory, db } = self;
        drop(db);
        let db = open(directory.path());
        Self { directory, db }
    }
}

fn open(directory: &Path) -> SharedDatabase {
    let mut sql = Connection::open(directory.join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &directory.join("data-log")).unwrap()
}

pub(super) struct PackageFixture {
    _directory: tempfile::TempDir,
    catalog: RuntimePluginCatalog,
    package: RuntimePluginPackage,
    resources: Vec<(String, String)>,
}

impl PackageFixture {
    pub fn new(resources: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let package_root = directory.path().join("literature");
        fs::create_dir_all(package_root.join("sql")).unwrap();
        fs::write(package_root.join("index.js"), "export {};").unwrap();
        fs::write(
            package_root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        for (name, sql) in resources {
            fs::write(package_root.join("sql").join(name), sql).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().into()));
        let package = catalog.discover().unwrap().packages.remove(0);
        Self {
            _directory: directory,
            catalog,
            package,
            resources: resources
                .iter()
                .map(|(name, sql)| ((*name).into(), (*sql).into()))
                .collect(),
        }
    }

    pub fn nal() -> Self {
        Self::new(&[("annotations.sql", READ_SQL), ("insert.sql", INSERT_SQL)])
    }

    pub fn resource(
        &self,
        name: &str,
        access: DataAccess,
        parameters: Vec<DataField>,
        columns: Vec<DataField>,
    ) -> AcceptedDataResource {
        let (_, sql) = self
            .resources
            .iter()
            .find(|(file, _)| file == name)
            .unwrap();
        let grant = DataResourceGrant {
            id: format!("literature.{}", name.trim_end_matches(".sql")),
            version: 1,
            sha256: format!("{:x}", Sha256::digest(sql.as_bytes())),
            access,
            parameters,
            columns,
        };
        AcceptedDataResource::load(
            &self.catalog,
            &self.package,
            "literature_library",
            &format!("sql/{name}"),
            grant,
        )
        .unwrap()
    }

    pub fn read(&self) -> AcceptedDataResource {
        self.resource(
            "annotations.sql",
            DataAccess::Read,
            vec![text()],
            vec![text(), text()],
        )
    }

    pub fn insert(&self) -> AcceptedDataResource {
        self.resource("insert.sql", DataAccess::Write, vec![text(); 3], vec![])
    }
}

pub(super) fn text() -> DataField {
    DataField {
        data_type: DataType::Text,
        nullable: false,
    }
}

pub(super) fn call(resource: &AcceptedDataResource, values: &[&str]) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments: values
            .iter()
            .map(|value| DataValue::Text((*value).into()))
            .collect(),
    }
}

pub(super) fn note(tx: &Transaction<'_>, title: &str) -> String {
    notes::create_page_with_paper_in(
        tx,
        notes::NotePageDraft {
            title: title.into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )
    .unwrap()
}

pub(super) fn counts(connection: &Connection) -> (u64, u64, u64) {
    connection
        .query_row(
            "select (select count(*) from note_pages), \
         (select count(*) from note_blocks), (select count(*) from nal_annotations)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}
