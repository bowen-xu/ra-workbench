use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::{RuntimePluginCatalog, literature_catalog::data::CatalogResources},
};
use research_workbench_native_sdk::{
    catalog::CatalogCreator,
    catalog_save::{
        CatalogDuplicatePolicy, CatalogSaveItem, CatalogSavePlan, CatalogSaveProvenance,
        PreparedDoi,
    },
};
use rusqlite::Connection;
use std::{fs, path::Path};

const READS: &[&str] = &[
    "list-active-ids",
    "search-fts-ids",
    "search-year-ids",
    "item-main",
    "item-creators",
    "item-tags",
    "item-attachments",
    "item-project-ids",
    "item-metadata-sources",
];
const WRITES: &[&str] = &[
    "item-upsert",
    "creators-delete",
    "creator-insert",
    "item-creator-insert",
    "doi-delete",
    "doi-insert",
    "tags-delete",
    "tag-insert",
    "item-tag-insert",
    "provenance-upsert",
    "search-delete",
    "search-insert",
    "duplicate-doi-ids",
    "duplicate-title-ids",
    "duplicate-insert",
];

pub(super) struct Fixture {
    pub directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: SaveResources,
    pub catalog: CatalogResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(directory.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &directory.path().join("data-log")).unwrap();
        let (resources, catalog) = package(directory.path(), "primary");
        Self {
            directory,
            db,
            resources,
            catalog,
        }
    }

    pub fn persist(&self, plan: &CatalogSavePlan) -> Result<CommittedCatalogRows, SaveFailure> {
        persist(
            &mut self.db.lock().unwrap(),
            &self.resources,
            &self.catalog,
            plan,
        )
    }

    pub fn reopen(self) -> Self {
        let Self {
            directory,
            db,
            resources,
            catalog,
        } = self;
        drop(db);
        let mut sql = Connection::open(directory.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &directory.path().join("data-log")).unwrap();
        Self {
            directory,
            db,
            resources,
            catalog,
        }
    }

    pub fn count(&self, table: &str) -> i64 {
        // Only compile-time synthetic test table names are passed here.
        self.db
            .lock()
            .unwrap()
            .query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
            .unwrap()
    }

    pub fn item(&self, id: &str, title: &str, year: Option<i32>) {
        self.db.lock().unwrap().execute(
            "insert into bibliographic_items(id,item_type,title,issued_year,created_at,updated_at) values(?1,'document',?2,?3,'before','before')",
            rusqlite::params![id,title,year],
        ).unwrap();
    }

    pub fn doi(&self, item: &str, id: &str) {
        self.db.lock().unwrap().execute(
            "insert into identifiers(id,item_id,kind,normalized_value,display_value) values(?1,?2,'doi','10.0/synthetic','10.0/Synthetic')",
            [id,item],
        ).unwrap();
    }
}

pub(super) fn package(directory: &Path, name: &str) -> (SaveResources, CatalogResources) {
    let root = directory.join(name).join("literature");
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(
        root.join("manifest.json"),
        r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(
        root.join("index.js"),
        format!("export const fixture = {name:?};"),
    )
    .unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in READS {
        let file = format!("catalog-{name}.v1.sql");
        fs::copy(original.join(&file), root.join("data").join(file)).unwrap();
    }
    for name in WRITES {
        let file = format!("catalog-save-{name}.v1.sql");
        fs::copy(original.join(&file), root.join("data").join(file)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(directory.join(name)));
    let selected = catalog.discover().unwrap().packages.remove(0);
    let writes = SaveResources::load(&catalog, &selected).unwrap();
    let reads = CatalogResources::load(&catalog, &selected).unwrap();
    assert_eq!(writes.package_key(), reads.package_key());
    (writes, reads)
}

/// A synthetic already-prepared plan. Core must persist these values, not plan again.
pub(super) fn plan(id: Option<&str>) -> CatalogSavePlan {
    CatalogSavePlan {
        item: CatalogSaveItem {
            id: id.map(str::to_owned),
            item_type: "journalArticle".into(),
            title: "Synthetic paper".into(),
            abstract_text: "Synthetic abstract".into(),
            issued_year: Some(2025),
            container_title: "Journal".into(),
            conference_name: "Conference".into(),
            proceedings_title: "Proceedings".into(),
            place: "Place".into(),
            volume: "2".into(),
            issue: "3".into(),
            pages: "4-5".into(),
            url: "https://example.invalid/paper".into(),
            language: "zh".into(),
            bibtex_key: "synthetic2025".into(),
            raw_bibtex: Some("@article{original}".into()),
        },
        creators: vec![CatalogCreator {
            family_name: "Family".into(),
            given_name: "Given".into(),
            literal_name: "".into(),
            role: "author".into(),
        }],
        doi: Some(PreparedDoi {
            display: "10.0/Synthetic".into(),
            normalized: "10.0/synthetic".into(),
        }),
        tags: vec!["First".into(), "Second".into()],
        provenance: CatalogSaveProvenance {
            fields: [
                "itemType",
                "title",
                "abstractText",
                "issuedYear",
                "containerTitle",
                "conferenceName",
                "proceedingsTitle",
                "place",
                "volume",
                "issue",
                "pages",
                "doi",
                "url",
                "language",
                "bibtexKey",
                "creators",
                "tags",
            ]
            .map(str::to_owned)
            .into(),
            source_kind: "user".into(),
            confirmed_by_user: true,
        },
        duplicates: CatalogDuplicatePolicy {
            normalized_title: "synthetic paper".into(),
            doi_reason: "doi".into(),
            doi_confidence: 1.0,
            title_reason: "normalized_title_year".into(),
            title_confidence: 0.72,
        },
    }
}
