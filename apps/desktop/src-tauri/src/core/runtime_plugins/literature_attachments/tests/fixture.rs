use super::*;
use crate::{data_interface::Database, runtime_plugins::RuntimePluginCatalog, storage::AppStore};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const PDF: &[u8] = b"%PDF-1.7\nSynthetic attachment\0\xff\n";

pub(super) fn text(value: &str) -> Value {
    Value::Text(value.into())
}

pub(super) struct Fixture {
    pub directory: tempfile::TempDir,
    pub store: AppStore,
    pub resources: AttachmentResources,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = open(directory.path());
        let selected = directory.path().join("packages/literature");
        fs::create_dir_all(selected.join("data")).unwrap();
        fs::write(
            selected.join("manifest.json"),
            r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(selected.join("entry.js"), "export {};\n").unwrap();
        let original = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        for entry in fs::read_dir(&original).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_str().unwrap();
            if name.starts_with("attachment-") || name == "catalog-save-duplicate-insert.v1.sql" {
                fs::copy(entry.path(), selected.join("data").join(name)).unwrap();
            }
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = AttachmentResources::load(&catalog, &package).unwrap();
        store
            .connection()
            .unwrap()
            .execute_batch(
                "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
             values('paper','document','Synthetic target','before','before'),
             ('other','document','Synthetic other','before','before');
             insert into literature_search(item_id,title,full_text)
             values('paper','Synthetic target','before'),('other','Synthetic other','before');",
            )
            .unwrap();
        Self {
            directory,
            store,
            resources,
        }
    }

    pub fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.directory.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    pub fn run(
        &self,
        request: AttachmentRequest,
        plan: Option<AttachmentPlanV1>,
    ) -> Result<AttachmentResult, AttachmentFailure> {
        execute(&self.store, &self.resources, request, plan)
    }

    pub fn attach(&self, item: &str, name: &str, bytes: &[u8]) -> CatalogAttachment {
        let source = self.source(name, bytes);
        let result = self
            .run(
                AttachmentRequest::Attach {
                    item_id: item.into(),
                    path: source.to_str().unwrap().into(),
                },
                None,
            )
            .unwrap();
        let AttachmentResult::Attachment(value) = result else {
            panic!("attachment result");
        };
        value
    }

    pub fn stored(&self, attachment: &str) -> PathBuf {
        let db = self.store.connection().unwrap();
        let relative: String = db.query_row(
            "select o.relative_path from content_objects o join attachments a on a.content_sha256=o.sha256 where a.id=?1",
            [attachment], |row| row.get(0)).unwrap();
        self.store.content_directory().join(relative)
    }

    pub fn remove(
        &self,
        attachment: &str,
    ) -> Result<
        research_workbench_native_sdk::attachments::AttachmentRemovalResult,
        AttachmentFailure,
    > {
        let result = self.run(
            AttachmentRequest::Remove {
                item_id: "paper".into(),
                attachment_id: attachment.into(),
            },
            None,
        )?;
        let AttachmentResult::Removed(value) = result else {
            panic!("removal result");
        };
        Ok(value)
    }

    pub fn read(&self, attachment: &str) -> Result<Vec<u8>, AttachmentFailure> {
        let result = self.run(
            AttachmentRequest::Read {
                attachment_id: attachment.into(),
            },
            None,
        )?;
        let AttachmentResult::Bytes(value) = result else {
            panic!("byte result");
        };
        Ok(value)
    }

    pub fn rows(&self, sql: &str) -> Vec<Vec<Value>> {
        // These tests pass only fixed synthetic projection statements. This
        // helper is not a production resource or Native SQL entry point.
        let db = self.store.connection().unwrap();
        let mut statement = db.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    pub fn prefix(&self) -> Vec<Vec<Vec<Value>>> {
        [
            "select * from attachments order by id",
            "select * from content_objects order by sha256",
            "select * from duplicate_candidates order by id",
            "select * from attachment_page_texts order by attachment_id,page_index",
            "select * from full_text_index_jobs order by attachment_id",
            "select * from reading_states order by item_id",
            "select * from literature_search order by item_id",
        ]
        .into_iter()
        .map(|sql| self.rows(sql))
        .collect()
    }

    pub fn reopen(self) -> Self {
        let Self {
            directory,
            store,
            resources,
        } = self;
        drop(store);
        let store = open(directory.path());
        Self {
            directory,
            store,
            resources,
        }
    }
}

fn open(directory: &Path) -> AppStore {
    let mut db = Connection::open(directory.join("attachments.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let db = Database::open(db, &directory.join("data-log")).unwrap();
    let objects = directory.join("objects");
    fs::create_dir_all(&objects).unwrap();
    AppStore::for_test_database(db, objects)
}
