use super::super::{CapturedUndo, UndoFailure, UndoResources, apply, capture};
use crate::{
    data_interface::Database,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_duplicates::tests::fixture::{Fixture as CoreFixture, PAGE},
    },
};
use research_workbench_native_sdk::duplicate_undo::{
    PreparedDuplicateUndoV1, UndoMergePrepareInputV1,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) const OPERATION: &str = "synthetic-undo-operation";
pub(crate) const CANDIDATE: &str = "candidate";

pub(crate) struct Fixture {
    pub data: CoreFixture,
    pub resources: UndoResources,
    pub catalog: Arc<RuntimePluginCatalog>,
    pub key: String,
    database_path: PathBuf,
    _package: tempfile::TempDir,
}

pub(crate) fn snapshot() -> Value {
    json!({"candidateId":CANDIDATE,"attachmentIds":[],"identifierIds":[],
        "anchorIds":[],"addedProjectIds":[],"addedTagIds":[],
        "addedProvenanceFields":[],"createdReadingState":false,"mergedArchivedAt":null})
}

pub(crate) fn classification() -> Value {
    json!({"beforeMemberships":[],"beforeAssignments":[],
        "afterMemberships":[],"afterAssignments":[]})
}

impl Fixture {
    pub fn new() -> Self {
        let data = CoreFixture::new();
        let database_path = {
            let db = data.db.lock().unwrap();
            PathBuf::from(db.path().expect("the fixture owns a file-backed SQL pair"))
        };
        data.candidate(CANDIDATE, 0.72);
        data.db
            .lock()
            .unwrap()
            .execute_batch(
                "update duplicate_candidates set decision='merge',decided_at='2026';
             update bibliographic_items set archived_at='merged-archive' where id='source';",
            )
            .unwrap();
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
            .join("../../../plugins/literature_library/native/data");
        for name in [
            "header",
            "collection-revision",
            "collection-collections",
            "collection-types",
            "collection-relations",
            "collection-memberships",
            "collection-assignments",
            "delete-statement",
            "delete-annotation",
            "increment-nal-revision",
            "delete-assignment",
            "delete-membership",
            "increment-collection-revision",
            "restore-anchor",
            "restore-reading-sources",
            "restore-attachment",
            "restore-reading-state",
            "restore-identifier",
            "delete-project",
            "delete-tag",
            "delete-provenance",
            "restore-archive",
            "reset-candidate",
            "mark-undone",
            "search-delete",
            "search-insert",
            "collection-scope",
            "statement-scope",
            "annotation-scope",
            "vector-equal",
        ] {
            let name = format!("duplicate-undo-{name}.v1.sql");
            fs::copy(source.join(&name), directory.join("data").join(name)).unwrap();
        }
        for name in ["revision", "concepts", "statements", "annotations", "rules"] {
            let name = format!("nal-bootstrap-state-{name}.v1.sql");
            fs::copy(source.join(&name), directory.join("data").join(name)).unwrap();
        }
        let catalog = Arc::new(RuntimePluginCatalog::new(Some(package.path().into())));
        let selected = catalog.discover().unwrap().packages.remove(0);
        let key = selected.package_key.clone();
        let resources = UndoResources::load(&catalog, &selected).unwrap();
        let result = Self {
            data,
            resources,
            catalog,
            key,
            database_path,
            _package: package,
        };
        result.seed_snapshot(&snapshot());
        result
    }

    pub fn seed_snapshot(&self, value: &Value) {
        self.seed_raw(&serde_json::to_string(value).unwrap());
    }

    pub fn seed_raw(&self, raw: &str) {
        self.data.db.lock().unwrap().execute(
            "insert into merge_operations(id,survivor_item_id,merged_item_id,snapshot_json,state,created_at)
             values(?1,'survivor','source',?2,'applied','2026')
             on conflict(id) do update set snapshot_json=excluded.snapshot_json,state='applied',undone_at=null",
            params![OPERATION, raw],
        ).unwrap();
    }

    pub fn capture(&self) -> CapturedUndo {
        capture(
            &self.data.db.lock().unwrap(),
            &self.resources,
            OPERATION.into(),
        )
        .unwrap()
    }

    pub fn prepared(&self, captured: &CapturedUndo) -> PreparedDuplicateUndoV1 {
        research_literature_library_native::duplicate_undo::prepare(&UndoMergePrepareInputV1 {
            snapshot_json: captured.snapshot_json.clone(),
        })
        .unwrap()
    }

    pub fn apply(&self) -> Result<(), UndoFailure> {
        let captured = self.capture();
        let plan = self.prepared(&captured);
        apply(
            &mut self.data.db.lock().unwrap(),
            &self.resources,
            &captured,
            &plan,
        )
    }

    pub fn seed_nested(&self) -> Value {
        self.data.db.lock().unwrap().execute_batch(
            "insert into collections(id,name,description) values('a','A',''),('b','B','');
             insert into collection_relations(id,source_id,target_id,type_id) values('r','a','b','related-field');
             insert into collection_memberships(item_id,collection_id) values('survivor','a'),('survivor','b');
             insert into item_relation_assignments(item_id,relation_id) values('survivor','r');",
        ).unwrap();
        let nal = self.seed_nal();
        let mut value = classification();
        value["afterMemberships"] = json!([
            {"itemId":"survivor","collectionId":"a"},
            {"itemId":"survivor","collectionId":"b"}]);
        value["afterAssignments"] = json!([{"itemId":"survivor","relationId":"r"}]);
        value["nal"] = nal;
        let mut outer = snapshot();
        outer["classification"] = value;
        outer
    }

    pub fn seed_nal(&self) -> Value {
        // Same typed fact as the snapshot, deliberately different JSON key order.
        self.data
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "insert into nal_statements(id,subject,predicate,origin)
             values('s','{\"id\":\"survivor\",\"kind\":\"paper\"}',
             '{\"id\":\"missing-concept\",\"kind\":\"concept\"}','human');",
            )
            .unwrap();
        self.data
            .db
            .lock()
            .unwrap()
            .execute(
                "insert into nal_annotations(id,statement_id,note_page_id) values('a','s',?1)",
                [PAGE],
            )
            .unwrap();
        json!({"before":[],"after":[{"id":"s","subject":{"kind":"paper","id":"survivor"},
            "predicate":{"kind":"concept","id":"missing-concept"},"origin":"human"}],
            "before_annotations":[],"after_annotations":[{"id":"a","targetKind":"statement",
            "targetId":"s","notePageId":PAGE}]})
    }

    pub fn seed_reading_source(&self) {
        let mut db = self.data.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let digest = "a".repeat(64);
        tx.execute(
            "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
            values(?1,'synthetic.pdf','application/pdf',1,'2026')",
            [&digest],
        )
        .unwrap();
        tx.execute(
            "insert into attachments(id,item_id,content_sha256,display_name,created_at)
            values('attachment','survivor',?1,'Synthetic.pdf','2026')",
            [&digest],
        )
        .unwrap();
        tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,created_at)
            values('anchor','survivor','attachment',?1,0,'2026')", [&digest]).unwrap();
        tx.execute("insert into reading_note_sources(block_id,page_id,note_type,color,current_anchor_id,
            paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,
            content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
            values('source-alias',?1,'annotation','#f4cf65','anchor','survivor','Synthetic survivor',
            'attachment','Synthetic.pdf',?2,0,'[]','Synthetic quote','','',1,'2026','2026')",
            params![PAGE,digest]).unwrap();
        tx.commit().unwrap();
    }

    pub fn state(&self) -> Value {
        let db = self.data.db.lock().unwrap();
        let mut value = serde_json::Map::new();
        for (name, sql) in [
            (
                "operation",
                "select json_array(id,snapshot_json,state,undone_at) from merge_operations order by id",
            ),
            (
                "candidate",
                "select json_array(id,decision,decided_at) from duplicate_candidates order by id",
            ),
            (
                "papers",
                "select json_array(id,title,archived_at,updated_at) from bibliographic_items order by id",
            ),
            (
                "collections",
                "select json_array(item_id,collection_id) from collection_memberships order by item_id,collection_id",
            ),
            (
                "assignments",
                "select json_array(item_id,relation_id) from item_relation_assignments order by item_id,relation_id",
            ),
            (
                "collectionRevision",
                "select json_array(revision) from collection_state",
            ),
            ("nalRevision", "select json_array(revision) from nal_state"),
            (
                "statements",
                "select json_array(id,subject,predicate,origin) from nal_statements order by rowid",
            ),
            (
                "annotations",
                "select json_array(id,concept_id,statement_id,note_page_id) from nal_annotations order by rowid",
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
                "attachments",
                "select json_array(id,item_id) from attachments order by id",
            ),
            (
                "search",
                "select json_array(item_id,title,creators,identifiers,tags,full_text) from literature_search order by item_id",
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
                "catalogSearchState",
                "select json_array(id,generation) from note_catalog_search_state",
            ),
            (
                "titleCache",
                "select json_array(page_id,body_title) from note_catalog_title_cache order by page_id",
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
            value.insert(name.into(), json!(rows(&db, sql)));
        }
        drop(db);
        value.insert("note".into(), self.data.note());
        Value::Object(value)
    }

    pub fn reopen(self) -> Self {
        Self {
            data: self.data.reopen(),
            ..self
        }
    }

    /// Recovery fences correctly reject normal shutdown. As in the original
    /// save fixture, release the complete old owner before opening its pair.
    pub fn reopen_fenced(mut self) -> Self {
        assert!(self.data.db.lock().is_err());
        assert_eq!(Arc::strong_count(&self.data.db), 1);
        let old_owner = self.data.db;
        drop(old_owner);
        // The partially moved Core fixture still owns its private TempDir.
        // Its original setup selected this exact sibling data-log directory.
        let mut sql = Connection::open(&self.database_path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        self.data.db =
            Database::open(sql, &self.database_path.parent().unwrap().join("data-log")).unwrap();
        self
    }
}

pub(crate) fn rows(db: &Connection, sql: &str) -> Vec<String> {
    db.prepare(sql)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
