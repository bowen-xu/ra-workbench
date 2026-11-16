use super::*;
use crate::data_interface::{Database, SharedDatabase};
use std::{fs, path::Path};

pub(super) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: BootstrapResources,
}
impl Fixture {
    pub fn reopen_after_recovery_fence(self) -> Self {
        let Self {
            directory,
            db,
            resources,
        } = self;
        assert!(db.lock().is_err(), "the original owner must remain fenced");
        assert_eq!(std::sync::Arc::strong_count(&db), 1);
        // A fenced owner rejects ordinary shutdown/materialization too. Release
        // its final complete owner, as the existing guarded_transaction recovery
        // regression does, then reopen the same SQL/log pair. Do not send SQL or
        // a cleanup request through that fence and call it a successful shutdown.
        drop(db);
        let mut connection = Connection::open(directory.path().join("bootstrap.sqlite3")).unwrap();
        crate::storage::migrate(&mut connection).unwrap();
        let db = Database::open(connection, &directory.path().join("data-log")).unwrap();
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut db = Connection::open(directory.path().join("bootstrap.sqlite3")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let db = Database::open(db, &directory.path().join("data-log")).unwrap();
        let package_root = directory.path().join("packages/literature");
        fs::create_dir_all(package_root.join("data")).unwrap();
        fs::write(
            package_root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(package_root.join("index.js"), "export {};").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        let mut count = 0;
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("nal-bootstrap-") {
                fs::copy(entry.path(), package_root.join("data").join(name)).unwrap();
                count += 1;
            }
        }
        assert_eq!(count, 45);
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = BootstrapResources::load(&catalog, &package).unwrap();
        Self {
            directory,
            db,
            resources,
        }
    }
    pub fn collection(&self) {
        self.db.lock().unwrap().execute("insert into collections(id,name,description) values('source','Original','Original description')",[]).unwrap();
    }
    pub fn term(&self, id: &str, normalized: &str, text: &str) {
        self.db.lock().unwrap().execute("insert into terms(id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at) values(?1,?2,?3,'en','human_original',1,'term-created','term-updated')",rusqlite::params![id,text,normalized]).unwrap();
    }
    pub fn concept(&self, id: &str, label: &str) {
        self.db.lock().unwrap().execute("insert into concepts(id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at) values(?1,?2,'explanation','disambiguation','domain','tradition','period','human_original',1,'concept-created','concept-updated')",rusqlite::params![id,label]).unwrap();
    }
}

pub(super) fn empty_preparation() -> (BootstrapProbePreparationV1, BootstrapIdentitiesV1) {
    (
        BootstrapProbePreparationV1 {
            concepts: vec![],
            term_keys: vec![],
            statement_count: 0,
            representation_render_version: "concept-identity-v1".into(),
            semantic_namespace: "accepted_concepts".into(),
            semantic_space: SourceProbe::Available { value: None },
            earliest_unavailable: None,
            terminal: None,
        },
        BootstrapIdentitiesV1 {
            concepts: vec![],
            generation_id: None,
            statement_ids: vec![],
        },
    )
}
pub(super) fn preparation(keys: &[&str]) -> (BootstrapProbePreparationV1, BootstrapIdentitiesV1) {
    let (mut preparation, mut identities) = empty_preparation();
    for (index, key) in keys.iter().enumerate() {
        let concept_key = if index == 0 {
            ConceptCallKeyV1::Membership {}
        } else {
            ConceptCallKeyV1::Tagged {}
        };
        preparation.concepts.push(ConceptIntentV1 {
            key: concept_key.clone(),
            label: (*key).into(),
            explanation: "Original".into(),
            term: TermValidationV1::Prepared {
                trimmed_text: (*key).into(),
                normalized_text: (*key).into(),
                language: "en".into(),
            },
        });
        preparation.term_keys.push(TermLookupKeyV1 {
            normalized_text: (*key).into(),
            language: "en".into(),
        });
        identities.concepts.push(ConceptIdentitySlotsV1 {
            key: concept_key,
            term_id: format!("term-{index}"),
            term_revision_id: format!("term-revision-{index}"),
            concept_id: format!("concept-{index}"),
            concept_revision_id: format!("concept-revision-{index}"),
            representation_id: format!("representation-{index}"),
            record_id: format!("record-{index}"),
            job_id: format!("job-{index}"),
        });
    }
    (preparation, identities)
}
