use super::*;
use crate::data_interface::{Database, SharedDatabase};
use crate::runtime_plugins::RuntimePluginCatalog;
use rusqlite::Connection;
use std::{fs, path::Path};

pub(crate) const PAGE: &str = "preserved-duplicate-core-note";

pub(crate) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: DuplicateResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = open(directory.path());
        let root = directory.path().join("packages/literature");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(root.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for name in [
            "duplicate-candidates-list",
            "duplicate-candidate-decide",
            "duplicate-merge-history",
        ] {
            let name = format!("{name}.v1.sql");
            fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = DuplicateResources::load(&catalog, &package).unwrap();
        {
            let mut db = db.lock().unwrap();
            db.execute_batch(
                "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
                values('survivor','journalArticle','Synthetic survivor','2026','2026'),
                ('source','journalArticle','Synthetic source','2026','2026');",
            )
            .unwrap();
            let tx = db.transaction().unwrap();
            crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Preserved duplicate Core note".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                Some(PAGE),
            )
            .unwrap();
            let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Preserved synthetic duplicate Core body","styles":[]}]});
            crate::notes::save_document_in(
                &tx,
                crate::notes::NoteDocumentSaveRequest {
                    document,
                    expected_revision: 0,
                    save_reason: "manual".into(),
                },
            )
            .unwrap();
            tx.commit().unwrap();
        }
        Self {
            directory,
            db,
            resources,
        }
    }

    pub fn candidate(&self, id: &str, confidence: f64) {
        self.db.lock().unwrap().execute(
            "insert into duplicate_candidates(id,left_item_id,right_item_id,reason,confidence,created_at)
             values(?1,'survivor','source','doi',?2,'2026')", rusqlite::params![id, confidence],
        ).unwrap();
    }

    pub fn note(&self) -> serde_json::Value {
        serde_json::to_value(
            crate::notes::load_document_value(&self.db.lock().unwrap(), PAGE).unwrap(),
        )
        .unwrap()
    }

    pub fn reopen(self) -> Self {
        let Self {
            directory,
            db,
            resources,
        } = self;
        db.shutdown().unwrap();
        drop(db);
        let db = open(directory.path());
        Self {
            directory,
            db,
            resources,
        }
    }
}

fn open(directory: &Path) -> SharedDatabase {
    let mut db = Connection::open(directory.join("duplicates.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    Database::open(db, &directory.join("data-log")).unwrap()
}
