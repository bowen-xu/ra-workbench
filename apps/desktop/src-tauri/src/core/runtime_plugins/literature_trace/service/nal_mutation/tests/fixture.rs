//! A two-call canned host for service ownership, never a Native domain engine.
use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    nal_bootstrap::*,
    nal_mutation::{
        CAPABILITY_ID, CAPABILITY_VERSION, NalMutationDecision, NalMutationPlan,
        NalMutationPreparation,
    },
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

pub(super) const PAGE: &str = "synthetic-mutation-core-note";
const BODY: &str = "Synthetic preserved mutation service Core body";
const ABSENT: &str = "synthetic-private-absent-annotation";

const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
if [ '__STOP_BEFORE_PROBE__' = yes ]; then
# This mode keeps the root alive before any Catalog request is polled. The
# test's Core R0 latch determines shutdown order, rather than an EOF/sleep race.
kill -STOP "$$"
exit 44
fi
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
case "$line" in *'"kind":"shutdown"'*) identity=${line%\"body\":*}; printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"; exit 0 ;; esac
case "$line" in *'"operation":"prepareNalBootstrapProbes"'*) ;; *) exit 42 ;; esac
reply '__PROBES__'
IFS= read -r line
case "$line" in *'"operation":"prepareNalMutation"'*) ;; *) exit 42 ;; esac
reply '__PREPARATION__'
if [ '__WRONG__' = yes ]; then
# Keep this fixture's owned root alive. This controls retirement evidence;
# it does not replace the older EOF/PermissionDenied regression fixture.
kill -STOP "$$"
exit 44
fi
IFS= read -r line || exit 0
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 43 ;; esac
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

pub(super) struct Fixture {
    pub _package: tempfile::TempDir,
    pub data: tempfile::TempDir,
    pub db: SharedDatabase,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub initial_identity: (String, String),
    input_directory: PathBuf,
    execution_directory: PathBuf,
}

impl Fixture {
    pub async fn new(wrong_second: bool) -> Self {
        Self::with_mode(wrong_second, false).await
    }

    pub async fn stopped_before_probe() -> Self {
        Self::with_mode(false, true).await
    }

    async fn with_mode(wrong_second: bool, stop_before_probe: bool) -> Self {
        let bootstrap = bootstrap();
        let preparation = if wrong_second {
            CatalogOutput::NalBootstrapPlan {
                outcome: Box::new(bootstrap),
            }
        } else {
            CatalogOutput::NalMutationPreparation {
                outcome: Box::new(NalMutationPreparation {
                    bootstrap,
                    decision: NalMutationDecision::Prepared {
                        mutation: Box::new(NalMutationPlan::UnlinkAnnotation { id: ABSENT.into() }),
                    },
                    rule_projection: vec![],
                }),
            }
        };
        let probes = CatalogOutput::NalBootstrapProbes {
            outcome: Box::new(BootstrapProbePreparationV1 {
                concepts: vec![],
                term_keys: vec![],
                statement_count: 0,
                representation_render_version: "concept-identity-v1".into(),
                semantic_namespace: "accepted_concepts".into(),
                semantic_space: SourceProbe::Skipped {},
                earliest_unavailable: None,
                terminal: None,
            }),
        };
        let mut script = HOST.replace("__WRONG__", if wrong_second { "yes" } else { "no" });
        script = script.replace(
            "__STOP_BEFORE_PROBE__",
            if stop_before_probe { "yes" } else { "no" },
        );
        for (marker, output) in [("__PROBES__", probes), ("__PREPARATION__", preparation)] {
            let output = serde_json::to_string(&output).unwrap();
            assert!(!output.contains('\''), "fixed synthetic shell literal");
            script = script.replace(marker, &output);
        }
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let manifest_file = root.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_file).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}));
        fs::write(manifest_file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        let mut resources = fs::read_dir(&source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| {
                name.to_str().is_some_and(|name| {
                    ["nal-bootstrap-", "nal-annotation-", "nal-mutation-"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                        && name.ends_with(".v1.sql")
                })
            })
            .collect::<Vec<_>>();
        resources.sort();
        assert_eq!(
            resources.len(),
            57,
            "45 startup + 3 annotation + 9 mutation resources"
        );
        for name in resources {
            fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let tx = sql.transaction().unwrap();
        crate::notes::create_page_with_paper_in(
            &tx,
            crate::notes::NotePageDraft {
                title: "Synthetic mutation lifecycle note".into(),
                parent_page_id: None,
                parent_block_position: None,
            },
            None,
            Some(PAGE),
        )
        .unwrap();
        let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
        document.blocks[0].content =
            json!({"kind":"inline","items":[{"type":"text","text":BODY,"styles":[]}]});
        crate::notes::save_document_in(
            &tx,
            crate::notes::NoteDocumentSaveRequest {
                document,
                expected_revision: 0,
                save_reason: "manual".into(),
            },
        )
        .unwrap();
        // Fixed synthetic starting revision. This is not a migration default.
        tx.execute("update nal_state set legacy_imported=0,relation_semantics_version=0,revision=0 where id=1", []).unwrap();
        tx.commit().unwrap();
        let db = Database::open(sql, &data.path().join("data-log")).unwrap();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let input_directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let execution_directory = staged.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let initial_identity = host.request_identity().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            _package: package,
            data,
            db,
            service,
            key,
            initial_identity,
            input_directory,
            execution_directory,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot(&self.db.lock().unwrap())
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.input_directory.exists());
        assert!(!self.execution_directory.exists());
    }

    pub fn cold_snapshot(self, fenced: bool) -> Snapshot {
        let Self {
            data, db, service, ..
        } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&db), 1);
        drop(service);
        if fenced {
            // Never send SQL/shutdown through the recovery fence. Releasing
            // the final owner closes the real SQL/log pair before reopening.
            assert!(db.lock().is_err());
        } else {
            db.shutdown().unwrap();
        }
        drop(db);
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let reopened = Database::open(sql, &data.path().join("data-log")).unwrap();
        let state = snapshot(&reopened.lock().unwrap());
        reopened.shutdown().unwrap();
        state
    }
}

pub(super) fn mutation() -> NalMutation {
    NalMutation::UnlinkAnnotation { id: ABSENT.into() }
}

fn bootstrap() -> BootstrapPreparationV1 {
    BootstrapPreparationV1 {
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
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Snapshot {
    pub flags: (i64, i64, i64),
    pub document: Value,
    pub counts: Vec<(&'static str, i64)>,
}

pub(super) fn snapshot(db: &Connection) -> Snapshot {
    let flags = db
        .query_row(
            "select legacy_imported,relation_semantics_version,revision from nal_state where id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let counts = [
        "terms",
        "concepts",
        "research_object_revisions",
        "semantic_representations",
        "embedding_records",
        "embedding_outbox_jobs",
        "nal_statements",
        "nal_annotations",
        "note_pages",
        "note_blocks",
        "note_revisions",
    ]
    .map(|table| {
        (
            table,
            db.query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap(),
        )
    })
    .into();
    Snapshot {
        flags,
        document: serde_json::to_value(crate::notes::load_document_value(db, PAGE).unwrap())
            .unwrap(),
        counts,
    }
}
