use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::{RuntimePluginCatalog, literature_nal_bootstrap as bootstrap},
};
use research_workbench_native_sdk::nal_bootstrap::*;
use std::{fs, path::Path, sync::Arc};

pub(super) const PAGE: &str = "assigned-annotation-page";
pub(super) const ANNOTATION: &str = "assigned-annotation";
pub(super) const ORIGINAL_PAGE: &str = "original-core-note";

pub(super) struct Fixture {
    directory: tempfile::TempDir,
    pub db: SharedDatabase,
    pub resources: AnnotationResources,
}

pub(super) struct PreparedCall {
    pub captured: bootstrap::PreparedBootstrapFacts,
    pub input: NalAnnotationPrepareInput,
    pub plan: NalAnnotationPreparation,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = open(directory.path());
        {
            let mut guard = db.lock().unwrap();
            guard
                .execute_batch(
                    "update nal_state set legacy_imported=1,relation_semantics_version=1;
                 insert into concepts(id,label,explanation,created_at,updated_at)
                 values('target','Synthetic target','','now','now');",
                )
                .unwrap();
            let tx = guard.transaction().unwrap();
            crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Original Core note".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                Some(ORIGINAL_PAGE),
            )
            .unwrap();
            let mut document = crate::notes::load_document_value(&tx, ORIGINAL_PAGE).unwrap();
            document.blocks[0].content = serde_json::json!({
                "kind":"inline","items":[{"type":"text","text":"Preserved original Core body","styles":[]}]
            });
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
        let package_root = directory.path().join("packages/literature");
        fs::create_dir_all(package_root.join("data")).unwrap();
        fs::write(
            package_root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(package_root.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        let mut copied = 0;
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name_text = name.to_string_lossy();
            if name_text.starts_with("nal-bootstrap-") || name_text.starts_with("nal-annotation-") {
                fs::copy(entry.path(), package_root.join("data").join(name)).unwrap();
                copied += 1;
            }
        }
        assert_eq!(copied, 48);
        let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
        let package = catalog.discover().unwrap().packages.remove(0);
        let resources = AnnotationResources::load(&catalog, &package).unwrap();
        Self {
            directory,
            db,
            resources,
        }
    }

    /// A hand-prepared completed-startup mechanism fixture, not Native policy.
    /// R0/R1 are the real owned captures and all expected rows are actual rows.
    pub fn call(&self, title: &str) -> PreparedCall {
        let probes = BootstrapProbePreparationV1 {
            concepts: vec![],
            term_keys: vec![],
            statement_count: 0,
            representation_render_version: "concept-identity-v1".into(),
            semantic_namespace: "accepted_concepts".into(),
            semantic_space: SourceProbe::Available { value: None },
            earliest_unavailable: None,
            terminal: None,
        };
        let identities = BootstrapIdentitiesV1 {
            concepts: vec![],
            generation_id: None,
            statement_ids: vec![],
        };
        let mut guard = self.db.lock().unwrap();
        let previous = bootstrap::capture_source(&mut guard, &self.resources.bootstrap).unwrap();
        let captured = bootstrap::capture_facts(
            &mut guard,
            &self.resources.bootstrap,
            &previous,
            &probes,
            &identities,
        )
        .unwrap();
        assert!(guard.is_autocommit());
        let SourceProbe::Available { value: raw_state } = &captured.source.raw_state else {
            panic!("actual raw state unavailable")
        };
        assert!(raw_state.relation_concept_ids.is_empty());
        let raw_state = raw_state.clone();
        let annotation = NalAnnotation {
            id: ANNOTATION.into(),
            target_kind: "concept".into(),
            target_id: "target".into(),
            note_page_id: PAGE.into(),
        };
        let plan = NalAnnotationPreparation {
            bootstrap: BootstrapPreparationV1 {
                concepts: vec![],
                import: ImportPreparationV1::Skipped {},
                repair: RepairPreparationV1::Skipped {},
                cleanup: CleanupPreparationV1::Skipped {},
                final_projection: StateProjectionV1::Prepared {
                    raw_state: raw_state.clone(),
                    rules: vec![],
                },
                terminal: None,
            },
            decision: NalAnnotationDecision::Create {
                annotation,
                raw_title: title.into(),
                link: NalAnnotationLinkPreparation::ReadActive {
                    annotations: raw_state.annotations,
                    target: NalAnnotationLinkTarget::Concept {
                        id: "target".into(),
                    },
                },
            },
        };
        let input = NalAnnotationPrepareInput {
            bootstrap: BootstrapInputV1 {
                source: captured.source.clone(),
                probes,
                facts: captured.facts.clone(),
                identities,
            },
            request: research_workbench_native_sdk::nal_annotation::NalAnnotationRequest {
                target_kind: "concept".into(),
                target_id: "target".into(),
                title: title.into(),
            },
            identities:
                research_workbench_native_sdk::nal_annotation_prepare::NalAnnotationIdentities {
                    page_id: PAGE.into(),
                    annotation_id: ANNOTATION.into(),
                },
        };
        PreparedCall {
            captured,
            input,
            plan,
        }
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let guard = self.db.lock().unwrap();
        let counts: Vec<_> = [
            "note_pages",
            "note_blocks",
            "note_revisions",
            "hbs_note_baselines",
            "nal_annotations",
            "note_page_catalog",
            "note_catalog_blocks",
        ]
        .map(|table| {
            // These identifiers are fixed test schema names, never input.
            let count: i64 = guard
                .query_row(&format!("select count(*) from {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (table, count)
        })
        .into();
        let revision: i64 = guard
            .query_row("select revision from nal_state where id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        serde_json::json!({"counts":counts,"revision":revision,"originalDocument":crate::notes::load_document_value(&guard,ORIGINAL_PAGE).unwrap()})
    }

    pub fn reopen_fenced(self) -> Self {
        let Self {
            directory,
            db,
            resources,
        } = self;
        assert!(db.lock().is_err());
        assert_eq!(Arc::strong_count(&db), 1);
        // Do not send SQL/shutdown through the recovery fence. Releasing the
        // final owner closes the unsettled SQL/log pair before cold reopen.
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
    let mut connection = rusqlite::Connection::open(directory.join("annotation.sqlite3")).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    Database::open(connection, &directory.join("data-log")).unwrap()
}
