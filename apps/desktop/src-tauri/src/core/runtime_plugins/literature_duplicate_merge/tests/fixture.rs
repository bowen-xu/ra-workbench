use super::super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog, literature_duplicates::tests::fixture::Fixture as CoreFixture,
};
use research_workbench_native_sdk::{
    duplicate_merge_prepare::MergePreparationV1, duplicate_review::MergeOperation,
};
use std::{fs, path::Path, sync::Arc};

pub(crate) const CANDIDATE: &str = "merge-candidate";
pub(crate) struct Fixture {
    pub(crate) data: CoreFixture,
    pub(crate) resources: MergeResources,
    pub(crate) catalog: Arc<RuntimePluginCatalog>,
    pub(crate) key: String,
    database_path: std::path::PathBuf,
    _package: tempfile::TempDir,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let data = CoreFixture::new();
        let database_path = data
            .db
            .lock()
            .unwrap()
            .path()
            .map(std::path::PathBuf::from)
            .unwrap();
        data.candidate(CANDIDATE, 0.72);
        let package = tempfile::tempdir().unwrap();
        let directory = package.path().join("literature");
        fs::create_dir_all(directory.join("data")).unwrap();
        fs::write(
            directory.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(directory.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name_text = name.to_string_lossy();
            if ["nal-bootstrap-", "duplicate-undo-", "duplicate-merge-"]
                .iter()
                .any(|p| name_text.starts_with(p))
            {
                fs::copy(entry.path(), directory.join("data").join(name)).unwrap();
            }
        }
        let catalog = Arc::new(RuntimePluginCatalog::new(Some(package.path().to_owned())));
        let selected = catalog.discover().unwrap().packages.remove(0);
        let resources = MergeResources::load(&catalog, &selected).unwrap();
        Self {
            data,
            resources,
            catalog,
            key: selected.package_key,
            database_path,
            _package: package,
        }
    }
    pub(crate) fn capture(&self) -> CapturedMerge {
        capture(
            &self.data.db.lock().unwrap(),
            &self.resources,
            CANDIDATE.into(),
            "survivor".into(),
        )
        .unwrap()
    }
    pub(crate) fn prepared(&self, captured: &CapturedMerge) -> MergePreparationV1 {
        assert_eq!(captured.input.survivor_item_id, "survivor");
        assert_eq!(captured.input.left_item_id, "survivor");
        assert_eq!(captured.input.right_item_id, "source");
        MergePreparationV1::Ready {
            input: captured.input.clone(),
            merged_item_id: "source".into(),
            operation_state: "applied".into(),
            created_reading_state: false,
        }
    }
    pub(crate) fn apply(&self) -> Result<MergeOperation, MergeFailure> {
        let captured = self.capture();
        let plan = self.prepared(&captured);
        apply(
            &mut self.data.db.lock().unwrap(),
            &self.resources,
            &captured,
            &plan,
        )
    }
    pub(crate) fn scalar<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.data
            .db
            .lock()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }
    pub(crate) fn collection(&self) {
        self.data.db.lock().unwrap().execute_batch("insert into collections(id,name,description) values('c','Original','Original description');
            insert into collection_memberships(item_id,collection_id) values('source','c');").unwrap();
    }

    pub(crate) fn reading_source(&self) {
        use crate::runtime_plugins::literature_duplicates::tests::fixture::PAGE;
        let digest = "a".repeat(64);
        let mut db = self.data.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic.pdf','application/pdf',1,'2026')",[&digest]).unwrap();
        tx.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','source',?1,'Synthetic.pdf','2026')",[&digest]).unwrap();
        tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,created_at) values('anchor','source','attachment',?1,0,'2026')",[&digest]).unwrap();
        tx.execute("insert into reading_note_sources(block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,
            attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
            values('source-alias',?1,'annotation','#f4cf65','anchor','source','Synthetic source','attachment','Synthetic.pdf',?2,0,'[]','Synthetic quote','','',1,'2026','2026')",
            rusqlite::params![PAGE,digest]).unwrap();
        tx.commit().unwrap();
    }

    pub(crate) fn derived(&self) -> serde_json::Value {
        let db = self.data.db.lock().unwrap();
        let mut state = serde_json::Map::new();
        for (name, sql) in [
            (
                "attachments",
                "select json_array(id,item_id) from attachments order by id",
            ),
            (
                "anchors",
                "select json_array(id,item_id,attachment_id) from source_anchors order by id",
            ),
            (
                "sources",
                "select json_array(block_id,page_id,current_anchor_id,paper_id_snapshot,paper_title_snapshot,updated_at) from reading_note_sources order by block_id",
            ),
            (
                "catalog",
                "select json_array(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) from note_page_catalog order by id",
            ),
            (
                "catalogState",
                "select json_array(id,generation) from note_page_catalog_state",
            ),
            (
                "searchState",
                "select json_array(id,generation) from note_catalog_search_state",
            ),
            (
                "metadata",
                "select json_array(page_id,search_text,folded_text) from note_catalog_metadata_text order by page_id",
            ),
            (
                "metadataSearch",
                "select json_array(rowid,folded_text) from note_catalog_metadata_search order by rowid",
            ),
            (
                "noteRevisions",
                "select json_array(id,page_id,page_revision,document_json,reason) from note_revisions order by id",
            ),
        ] {
            let values: Vec<String> = db
                .prepare(sql)
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            state.insert(name.into(), serde_json::json!(values));
        }
        serde_json::Value::Object(state)
    }

    pub(crate) fn reopen_fenced(mut self) -> Self {
        assert!(self.data.db.lock().is_err());
        let path = self.database_path.clone();
        assert_eq!(Arc::strong_count(&self.data.db), 1);
        let old = self.data.db;
        drop(old);
        let mut sql = rusqlite::Connection::open(&path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        self.data.db =
            crate::data_interface::Database::open(sql, &path.parent().unwrap().join("data-log"))
                .unwrap();
        self
    }
}
