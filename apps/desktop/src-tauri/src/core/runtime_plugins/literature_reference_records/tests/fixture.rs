use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::RuntimePluginCatalog,
};
use rusqlite::Connection;
use std::{fs, path::Path};

const PAGE: &str = "reference-record-core-note";

pub(super) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: ReferenceResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = open(directory.path());
        let package_root = directory.path().join("packages/literature");
        fs::create_dir_all(package_root.join("data")).unwrap();
        fs::write(
            package_root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(package_root.join("index.js"), "export {};\n").unwrap();
        let original = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for key in [
            "ids",
            "record",
            "anchor-belongs",
            "matched-exists",
            "insert",
            "header",
            "update",
            "revision",
        ] {
            let name = format!("reference-records-{key}.v1.sql");
            fs::copy(original.join(&name), package_root.join("data").join(name)).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = ReferenceResources::load(&catalog, &package).unwrap();
        {
            let mut db = db.lock().unwrap();
            db.execute_batch(
                "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
                 values('citing','journalArticle','Synthetic citing paper','before','before'),
                 ('cited','journalArticle','Synthetic cited paper','before','before'),
                 ('other','journalArticle','Synthetic other paper','before','before');
                 insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
                 values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'aa/synthetic.pdf','application/pdf',10,'before');
                 insert into attachments(id,item_id,content_sha256,display_name,created_at)
                 values('attachment','citing',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'synthetic.pdf','before'),('other-attachment','other',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'synthetic-other.pdf','before');
                 insert into source_anchors
                 (id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at)
                 values('reference-anchor','citing','attachment',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 2,'Synthetic bibliography quote','before'),
                 ('usage-anchor','citing','attachment',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 0,'Synthetic use of the cited method','before'),
                 ('other-anchor','other','other-attachment',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 0,'Synthetic other-paper quote','before');",
            )
            .unwrap();
            let transaction = db.transaction().unwrap();
            crate::notes::create_page_with_paper_in(
                &transaction,
                crate::notes::NotePageDraft {
                    title: "Synthetic Core note before reference capture".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                Some("citing"),
                Some(PAGE),
            )
            .unwrap();
            let mut document = crate::notes::load_document_value(&transaction, PAGE).unwrap();
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{
                "type":"text","text":"Core Note and anchors precede reference save","styles":[]
            }]});
            crate::notes::save_document_in(
                &transaction,
                crate::notes::NoteDocumentSaveRequest {
                    document,
                    expected_revision: 0,
                    save_reason: "manual".into(),
                },
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        Self {
            directory,
            db,
            resources,
        }
    }

    pub fn save(&self, draft: ReferenceRecordDraft) -> Result<ReferenceRecord, ReferenceFailure> {
        save(&mut self.db.lock().unwrap(), &self.resources, &plan(draft))
    }

    pub fn records(&self, paper: &str) -> Result<Vec<ReferenceRecord>, ReferenceFailure> {
        list(&self.db.lock().unwrap(), &self.resources, paper)
    }

    pub fn count(&self, table: &str) -> i64 {
        // Only compile-time synthetic table names are passed by these tests.
        self.db
            .lock()
            .unwrap()
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn prefix(&self) -> serde_json::Value {
        let db = self.db.lock().unwrap();
        let note =
            serde_json::to_value(crate::notes::load_document_value(&db, PAGE).unwrap()).unwrap();
        let mut statement = db.prepare(
            "select id,item_id,attachment_id,page_index,quote_exact from source_anchors order by id",
        ).unwrap();
        let anchors = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        serde_json::json!({"note":note,"anchors":anchors})
    }

    pub fn reopen(self) -> Self {
        let Self {
            directory,
            db,
            resources,
        } = self;
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
    let mut sql = Connection::open(directory.join("references.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &directory.join("data-log")).unwrap()
}

pub(super) fn draft() -> ReferenceRecordDraft {
    ReferenceRecordDraft {
        id: None,
        citing_paper_id: "citing".into(),
        source_anchor_id: "reference-anchor".into(),
        raw_text: "  Synthetic bibliography reference  ".into(),
        fields: Vec::new(),
        parse_state: "unparsed".into(),
        identity_state: "unverified".into(),
        matched_item_id: None,
        semantic_state: "unreviewed".into(),
        usage_kind: None,
        usage_summary: String::new(),
        usage_anchor_id: None,
        expected_revision: None,
        change_reason: "  manual_capture  ".into(),
    }
}

/// A prepared data fixture, not another domain validator or host implementation.
pub(super) fn plan(draft: ReferenceRecordDraft) -> ReferenceRecordPlanV1 {
    ReferenceRecordPlanV1 {
        raw_text: draft.raw_text.trim().into(),
        usage_summary: draft.usage_summary.trim().into(),
        change_reason: draft.change_reason.trim().into(),
        fields_json: serde_json::to_string(&draft.fields).unwrap(),
        snapshot_json: serde_json::to_string(&draft).unwrap(),
        draft,
    }
}
