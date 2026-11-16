use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use research_workbench_native_sdk::nal_bootstrap::*;
use rusqlite::Connection;
use serde_json::json;
use std::{fs, path::PathBuf};

// The existing service test's finite frame/sidecar mechanism. Both domain
// outputs below are canned empty-collection facts, never a shadow planner.
const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
reply() {
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' "$1" > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
}
IFS= read -r line
case "$line" in *'"operation":"prepareNalBootstrapProbes"'*) ;; *) exit 42 ;; esac
reply '__PROBES__'
IFS= read -r line
case "$line" in *'"operation":"prepareNalBootstrap"'*) ;; *) exit 42 ;; esac
reply '__PLAN__'
IFS= read -r ignored
"#;

pub(super) struct Fixture {
    pub _package: tempfile::TempDir,
    pub data: tempfile::TempDir,
    pub db: SharedDatabase,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub input_directory: PathBuf,
    pub execution_directory: PathBuf,
    pub executable: PathBuf,
}

enum FirstReply {
    Probes,
    WrongOperation,
    ImpossibleCount,
}

impl Fixture {
    pub async fn two_phases() -> Self {
        Self::new(FirstReply::Probes).await
    }

    pub async fn wrong_first_operation() -> Self {
        Self::new(FirstReply::WrongOperation).await
    }

    pub async fn impossible_count() -> Self {
        Self::new(FirstReply::ImpossibleCount).await
    }

    async fn new(first: FirstReply) -> Self {
        let plan = CatalogOutput::NalBootstrapPlan {
            outcome: Box::new(BootstrapPreparationV1 {
                concepts: vec![],
                import: ImportPreparationV1::Prepared(Box::new(PreparedImportV1 {
                    membership: None,
                    tagged: None,
                    collections: vec![],
                    linked_predicate: None,
                    type_predicates: vec![],
                    related_field_probe: None,
                    subfield_predicate: None,
                    linked_statements: vec![],
                    relation_statements: vec![],
                    membership_statements: vec![],
                    assignment_statements: vec![],
                })),
                repair: RepairPreparationV1::Prepared {
                    relation: None,
                    rewrites: vec![],
                },
                cleanup: CleanupPreparationV1::Skipped {},
                final_projection: StateProjectionV1::Prepared {
                    raw_state: RawNalStateV1 {
                        revision: 2,
                        concepts: vec![],
                        statements: vec![],
                        annotations: vec![],
                        relation_concept_ids: vec![],
                    },
                    rules: vec![],
                },
                terminal: None,
            }),
        };
        let probes = CatalogOutput::NalBootstrapProbes {
            outcome: Box::new(BootstrapProbePreparationV1 {
                concepts: vec![],
                term_keys: vec![],
                statement_count: if matches!(first, FirstReply::ImpossibleCount) {
                    usize::MAX
                } else {
                    0
                },
                representation_render_version: "concept-identity-v1".into(),
                semantic_namespace: "accepted_concepts".into(),
                semantic_space: SourceProbe::Skipped {},
                earliest_unavailable: None,
                terminal: None,
            }),
        };
        let plan = serde_json::to_string(&plan).unwrap();
        let probes = if matches!(first, FirstReply::WrongOperation) {
            plan.clone()
        } else {
            serde_json::to_string(&probes).unwrap()
        };
        assert!(
            !probes.contains('\'') && !plan.contains('\''),
            "fixed synthetic shell literals"
        );
        let script = HOST
            .replace("__PROBES__", &probes)
            .replace("__PLAN__", &plan);
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let manifest_path = root.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}));
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        let mut resources = fs::read_dir(&source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| {
                name.to_str().is_some_and(|name| {
                    name.starts_with("nal-bootstrap-") && name.ends_with(".v1.sql")
                })
            })
            .collect::<Vec<_>>();
        resources.sort();
        assert_eq!(
            resources.len(),
            45,
            "fixed bootstrap package resource footprint"
        );
        for name in resources {
            fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        // Migration 0171 increments the fresh revision to 1. This lifecycle
        // fixture explicitly starts its synthetic singleton at 0 so the two
        // canned phases have the fixed final revision 2; no product default changes.
        sql.execute("update nal_state set revision=0 where id=1", [])
            .unwrap();
        let db = Database::open(sql, &data.path().join("data-log")).unwrap();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let input_directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let execution_directory = staged.directory.path().to_owned();
        let executable = staged.executable.clone();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            _package: package,
            data,
            db,
            service,
            key,
            input_directory,
            execution_directory,
            executable,
        }
    }

    pub fn snapshot(&self) -> ((i64, i64, i64), Vec<i64>, u64) {
        let db = self.db.lock().unwrap();
        let flags = flags(&db);
        let counts = domain_counts(&db);
        (flags, counts, db.total_changes())
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.input_directory.exists());
        assert!(!self.execution_directory.exists());
        assert!(!self.executable.exists());
    }
}

pub(super) fn flags(db: &Connection) -> (i64, i64, i64) {
    db.query_row(
        "select legacy_imported,relation_semantics_version,revision from nal_state where id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .unwrap()
}

pub(super) fn domain_counts(db: &Connection) -> Vec<i64> {
    [
        "terms",
        "concepts",
        "research_object_revisions",
        "semantic_representations",
        "embedding_spaces",
        "embedding_index_generations",
        "embedding_records",
        "embedding_outbox_jobs",
        "nal_statements",
        "nal_annotations",
        "nal_relation_rules",
    ]
    .iter()
    .map(|table| {
        db.query_row(&format!("select count(*) from {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    })
    .collect()
}
