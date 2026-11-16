//! Real deployed migrations and SQL/log owner; only finite preparation is canned.
use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::RuntimePluginCatalog,
};
use rusqlite::{Connection, params};
use std::{fs, path::Path};

const PAGE: &str = "reading-unit-adapter-core-note";

pub(super) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: ReadingUnitResources,
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
        let original = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        // Copy immutable source files, but the production loader accepts only
        // its eight fixed grants. This fixture supplies no alternate SQL executor.
        for entry in fs::read_dir(original).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "sql") {
                fs::copy(entry.path(), root.join("data").join(entry.file_name())).unwrap();
            }
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = ReadingUnitResources::load(&catalog, &package).unwrap();
        {
            let mut sql = db.lock().unwrap();
            let tx = sql.transaction().unwrap();
            crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Synthetic Core note before reading-unit save".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                Some(PAGE),
            )
            .unwrap();
            let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{
                "type":"text","text":"Core Note remains independent of reading-unit writes","styles":[]
            }]});
            crate::notes::save_document_in(
                &tx,
                crate::notes::NoteDocumentSaveRequest {
                    document,
                    expected_revision: 0,
                    save_reason: "manual".into(),
                },
            )
            .unwrap();
            insert_source(&tx, "a");
            tx.commit().unwrap();
        }
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn apply(
        &self,
        value: ReadingUnitPreparationV1,
    ) -> Result<ReadingUnitResult, ReadingUnitFailure> {
        apply(&mut self.db.lock().unwrap(), &self.resources, value)
    }
    pub fn save(
        &self,
        value: ReadingUnitPreparationV1,
    ) -> Result<StructuredReadingUnit, ReadingUnitFailure> {
        match self.apply(value)? {
            ReadingUnitResult::Unit(v) => Ok(*v),
            _ => panic!("save"),
        }
    }
    pub fn note(&self) -> serde_json::Value {
        serde_json::to_value(
            crate::notes::load_document_value(&self.db.lock().unwrap(), PAGE).unwrap(),
        )
        .unwrap()
    }
    pub fn state(&self) -> serde_json::Value {
        let db = self.db.lock().unwrap();
        let mut values = serde_json::Map::new();
        for (key, query) in [
            (
                "units",
                "select json_array(id,project_id,paper_id,unit_kind,title,body,provenance_kind,epistemic_role,revision,created_at,updated_at,reverted_at) from structured_reading_units order by id",
            ),
            (
                "evidence",
                "select json_array(id,reading_unit_id,anchor_id,stance,rationale,created_at) from reading_unit_evidence_links order by id",
            ),
            (
                "revisions",
                "select json_array(id,object_kind,object_id,revision,snapshot_json,editor_kind,change_reason,created_at) from research_object_revisions order by id",
            ),
        ] {
            let rows = db
                .prepare(query)
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            values.insert(key.into(), serde_json::json!(rows));
        }
        serde_json::Value::Object(values)
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
    let mut sql = Connection::open(directory.join("reading-units.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &directory.join("data-log")).unwrap()
}
pub(super) fn draft() -> StructuredReadingUnitDraft {
    StructuredReadingUnitDraft {
        id: None,
        project_id: None,
        paper_id: Some("paper-a".into()),
        unit_kind: "other".into(),
        title: "Title".into(),
        body: "Body".into(),
        epistemic_role: "working_note".into(),
        anchor_id: None,
        evidence_stance: None,
        evidence_rationale: None,
        expected_revision: None,
        change_reason: None,
    }
}
pub(super) fn plan(draft: StructuredReadingUnitDraft) -> ReadingUnitPreparationV1 {
    // Fixed canned contract data, not a Native engine. Test values are already valid.
    let title = draft.title.trim().to_owned();
    let body = draft.body.trim().to_owned();
    let evidence_stance = draft.evidence_stance.clone().unwrap_or("context".into());
    let evidence_rationale = draft
        .evidence_rationale
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_owned();
    let rules = ReadingUnitRulesV1 {
        provenance_kind: "human_original".into(),
        object_kind: "reading_unit".into(),
        editor_kind: "human".into(),
        change_reason: draft.change_reason.clone().unwrap_or("manual_save".into()),
        initial_revision: 1,
        new_expected_rejection: (draft.id.is_none() && draft.expected_revision.is_some())
            .then_some(ReadingUnitRejectionV1::NewExpectedRevision),
    };
    ReadingUnitPreparationV1::Ready {
        plan: Box::new(ReadingUnitPlanV1::Save {
            draft: Box::new(draft),
            title,
            body,
            evidence_stance,
            evidence_rationale,
            rules: Box::new(rules),
        }),
    }
}
fn insert_source(connection: &Connection, suffix: &str) -> (String, String) {
    let paper_id = format!("paper-{suffix}");
    let attachment_id = format!("attachment-{suffix}");
    let anchor_id = format!("anchor-{suffix}");
    let hash = match suffix {
        "a" => "a".repeat(64),
        _ => "b".repeat(64),
    };
    connection
        .execute(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values (?1, 'journalArticle', ?2, 'now', 'now')",
            rusqlite::params![paper_id, format!("Paper {suffix}")],
        )
        .expect("paper");
    connection
        .execute(
            "insert into content_objects
             (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'application/pdf', 10, 'now')",
            params![hash, format!("aa/{suffix}.pdf")],
        )
        .expect("content");
    connection
        .execute(
            "insert into attachments
             (id, item_id, content_sha256, display_name, created_at)
             values (?1, ?2, ?3, ?4, 'now')",
            params![attachment_id, paper_id, hash, format!("{suffix}.pdf")],
        )
        .expect("attachment");
    connection
        .execute(
            "insert into source_anchors
             (id, item_id, attachment_id, content_sha256, page_index, page_label,
              rects_json, quote_exact, quote_prefix, quote_suffix, created_at)
             values (?1, ?2, ?3, ?4, 0, '1', '[]', ?5, '', '', 'now')",
            params![
                anchor_id,
                paper_id,
                attachment_id,
                hash,
                format!("Calibration usage {suffix}")
            ],
        )
        .expect("anchor");
    (paper_id, anchor_id)
}
