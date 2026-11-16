use super::super::*;
use crate::data_interface::{Database, SharedDatabase};
use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub(super) const TRACE_ID: &str = "synthetic-trace";
pub(super) const QUERY: &str = "Synthetic retrieval query";
pub(super) const FUTURE: &str = "2999-01-01T00:00:00.000Z";
pub(super) const PAST: &str = "2000-01-01T00:00:00.000Z";

pub(super) struct DatabaseFixture {
    pub directory: tempfile::TempDir,
    pub db: SharedDatabase,
}

impl DatabaseFixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = open(directory.path());
        Self { directory, db }
    }

    pub fn insert_trace(&self, body: &str, expires_at: &str) {
        self.db
            .lock()
            .unwrap()
            .execute(
                "insert into concept_retrieval_traces
             (id,paper_id,query_sha256,status,config_json,context_text,context_sha256,
              seed_count,edge_count,source_count,external_paper_count,created_at,expires_at)
             values(?1,null,?2,'previewed','{}',?3,?4,0,0,0,0,'now',?5)",
                params![TRACE_ID, hash(QUERY), body, hash(body), expires_at],
            )
            .unwrap();
    }

    pub fn capture(&self, package: &PackageFixture, page_id: &str) -> PreparedTrace {
        let selection = RetrievalSelection {
            trace_id: TRACE_ID.into(),
            query: QUERY.into(),
        };
        // The real caller's original selection validation precedes trace-read.
        selection.validate().unwrap();
        let mut connection = self.db.lock().unwrap();
        let tx = connection.transaction().unwrap();
        let prepared =
            capture_note_trace_in(&tx, package.resources(), &selection, page_id).unwrap();
        tx.commit().unwrap();
        prepared
    }

    pub fn reopen(self) -> Self {
        let Self { directory, db } = self;
        drop(db);
        let db = open(directory.path());
        Self { directory, db }
    }

    pub fn status(&self) -> String {
        status(&self.db.lock().unwrap())
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
}

impl PackageFixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("literature");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("index.js"), "export {};").unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        for name in ["read", "expire", "mark-sent"] {
            let filename = format!("trace-{name}.v1.sql");
            fs::copy(
                resource_directory().join(&filename),
                root.join("data").join(filename),
            )
            .unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().into()));
        let package = catalog.discover().unwrap().packages.remove(0);
        Self {
            _directory: directory,
            catalog,
            package,
        }
    }

    pub fn resources(&self) -> TraceResources {
        TraceResources::load(&self.catalog, &self.package).unwrap()
    }
}

pub(super) fn resource_directory() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature")
}

pub(super) fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub(super) fn ready() -> TraceDecision {
    TraceDecision::Ready {
        mark_sent: MarkSentCondition::ORIGINAL,
    }
}

pub(super) fn immediate(connection: &mut Connection) -> rusqlite::Transaction<'_> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap()
}

pub(super) fn status(connection: &Connection) -> String {
    connection
        .query_row(
            "select status from concept_retrieval_traces where id=?1",
            [TRACE_ID],
            |row| row.get(0),
        )
        .unwrap()
}

pub(super) fn core_note(
    tx: &Transaction<'_>,
    title: &str,
) -> Result<String, crate::notes::NotesError> {
    crate::notes::create_page_with_paper_in(
        tx,
        crate::notes::NotePageDraft {
            title: title.into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )
}
