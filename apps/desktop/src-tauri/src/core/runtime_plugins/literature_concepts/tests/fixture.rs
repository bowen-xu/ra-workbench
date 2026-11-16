//! Real deployed migrations and SQL/log owner; only Native's finite reply is canned.
use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::RuntimePluginCatalog,
};
use research_workbench_native_sdk::concepts::{
    ConceptDraft, ConceptPlanV1, ConceptRulesV1, ConceptTermPlanV1, NewConceptDetails,
};
use rusqlite::Connection;
use std::{fs, path::Path};

const PAGE: &str = "concept-adapter-core-note";

pub(super) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: ConceptResources,
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
        // its 33 fixed grants. This fixture supplies no alternate SQL executor.
        for entry in fs::read_dir(original).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "sql") {
                fs::copy(entry.path(), root.join("data").join(entry.file_name())).unwrap();
            }
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = ConceptResources::load(&catalog, &package).unwrap();
        {
            let mut sql = db.lock().unwrap();
            let tx = sql.transaction().unwrap();
            crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Synthetic Core note before concept save".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                Some(PAGE),
            )
            .unwrap();
            let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
            document.blocks[0].content = serde_json::json!({"kind":"inline","items":[{
                "type":"text","text":"Core Note remains independent of concept writes","styles":[]
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
            tx.commit().unwrap();
        }
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn apply(&self, value: ConceptPreparationV1) -> Result<ConceptResult, ConceptFailure> {
        apply(&mut self.db.lock().unwrap(), &self.resources, value)
    }
    pub fn save(&self, value: ConceptPreparationV1) -> Result<Concept, ConceptFailure> {
        match self.apply(value)? {
            ConceptResult::Concept(value) => Ok(*value),
            ConceptResult::Concepts(_) => panic!("save returned a list"),
        }
    }
    pub fn count(&self, table: &str) -> i64 {
        // Every caller supplies a compile-time synthetic table name.
        self.db
            .lock()
            .unwrap()
            .query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
            .unwrap()
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
        for (key, sql) in [
            (
                "terms",
                "select json_array(id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at) from terms order by id",
            ),
            (
                "concepts",
                "select json_array(id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at,deleted_at) from concepts order by id",
            ),
            (
                "links",
                "select json_array(concept_id,term_id,relation_kind) from concept_terms order by concept_id,term_id",
            ),
            (
                "revisions",
                "select json_array(id,object_kind,object_id,revision,snapshot_json,editor_kind,change_reason,created_at) from research_object_revisions order by id",
            ),
            (
                "representations",
                "select json_array(id,subject_id,subject_revision,language,render_version,rendered_text,input_sha256,created_at) from semantic_representations order by id",
            ),
            (
                "records",
                "select json_array(id,representation_id,embedding_space_id,index_generation_id,lance_row_id,status) from embedding_records order by id",
            ),
            (
                "jobs",
                "select json_array(id,embedding_record_id,attempt,status,idempotency_key) from embedding_outbox_jobs order by id",
            ),
        ] {
            let rows = db
                .prepare(sql)
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
        // Drop, rather than admission/shutdown on a fenced owner, releases the
        // original SQL/log pair and rolls back an unsettled SQLite transaction.
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
    let mut sql = Connection::open(directory.join("concepts.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &directory.join("data-log")).unwrap()
}

pub(super) fn draft() -> ConceptDraft {
    ConceptDraft {
        id: None,
        preferred_term: "Synthetic concept".into(),
        language: Some("en".into()),
        label: "Synthetic concept".into(),
        explanation: "Synthetic explanation".into(),
        disambiguation: String::new(),
        domain: "Synthetic research".into(),
        tradition: String::new(),
        period: String::new(),
        expected_revision: None,
        change_reason: None,
    }
}

pub(super) fn plan(value: ConceptDraft) -> ConceptPreparationV1 {
    // Explicit finite canned protocol data, not a Native/domain implementation.
    // Test drafts already contain normalized values; pure rules are independently
    // tested in the canonical Native crate and actual-host acceptance is separate.
    let term = ConceptTermPlanV1 {
        text: value.preferred_term.clone(),
        normalized_text: "synthetic concept".into(),
        language: "en".into(),
        rejection: None,
    };
    let details = NewConceptDetails {
        label: value.label.clone(),
        explanation: value.explanation.clone(),
        disambiguation: value.disambiguation.clone(),
        domain: value.domain.clone(),
        tradition: value.tradition.clone(),
        period: value.period.clone(),
        change_reason: value.change_reason.clone(),
    };
    let rules = ConceptRulesV1 {
        provenance_kind: "human_original".into(),
        editor_kind: "human".into(),
        initial_revision: 1,
        term_object_kind: "term".into(),
        concept_object_kind: "concept".into(),
        term_reason: "manual_create".into(),
        concept_reason: "manual_create".into(),
        new_expected_rejection: None,
    };
    ConceptPreparationV1::Ready {
        plan: Box::new(ConceptPlanV1::Save {
            draft: Box::new(value),
            details: Box::new(details),
            term,
            rules,
        }),
    }
}
pub(super) fn parts(
    value: &mut ConceptPreparationV1,
) -> (&mut ConceptTermPlanV1, &mut ConceptRulesV1) {
    let ConceptPreparationV1::Ready { plan } = value else {
        panic!("ready fixture")
    };
    let ConceptPlanV1::Save { term, rules, .. } = plan.as_mut() else {
        panic!("save fixture")
    };
    (term, rules)
}
pub(super) fn search(query: &str, normalized: &str, pattern: &str) -> ConceptPreparationV1 {
    ConceptPreparationV1::Ready {
        plan: Box::new(ConceptPlanV1::Search {
            query: query.into(),
            normalized: normalized.into(),
            pattern: pattern.into(),
        }),
    }
}
