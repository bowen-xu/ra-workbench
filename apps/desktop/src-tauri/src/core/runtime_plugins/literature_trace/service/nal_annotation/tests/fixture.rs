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
    nal_annotation::{NalAnnotation, NalAnnotationRequest},
    nal_annotation_prepare::{
        CAPABILITY_ID, CAPABILITY_VERSION, NalAnnotationDecision, NalAnnotationPreparation,
        NalAnnotationValidation,
    },
    nal_bootstrap::*,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{fs, path::PathBuf};

pub(super) const PAGE: &str = "synthetic-existing-note";
const TARGET: &str = "synthetic-existing-statement";
const ANNOTATION: &str = "synthetic-existing-annotation";
const SUBJECT: &str = r#"{"kind":"concept","id":"synthetic-subject"}"#;
const PREDICATE: &str = r#"{"kind":"concept","id":"synthetic-predicate"}"#;

// Reuse the existing finite frame/sidecar fixture. Replies are canned typed
// empty-import facts; this script does not implement a domain engine or owner.
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
case "$line" in *'"kind":"shutdown"'*) identity=${line%\"body\":*}; printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"; exit 0 ;; esac
case "$line" in *'"operation":"validateNalAnnotation"'*) ;; *) exit 42 ;; esac
reply '__VALIDATION__'
if [ '__INVALID__' = no ]; then
IFS= read -r line
case "$line" in *'"operation":"prepareNalBootstrapProbes"'*) ;; *) exit 42 ;; esac
reply '__PROBES__'
IFS= read -r line
case "$line" in *'"operation":"prepareNalAnnotation"'*) ;; *) exit 42 ;; esac
reply '__PREPARATION__'
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
    pub input_directory: PathBuf,
    pub execution_directory: PathBuf,
    pub executable: PathBuf,
}

enum Reply {
    Invalid,
    WrongThirdOperation,
    Existing,
}

impl Fixture {
    pub async fn invalid() -> Self {
        Self::new(Reply::Invalid).await
    }
    pub async fn wrong_third_operation() -> Self {
        Self::new(Reply::WrongThirdOperation).await
    }
    pub async fn existing() -> Self {
        Self::new(Reply::Existing).await
    }

    async fn new(reply: Reply) -> Self {
        let bootstrap = bootstrap();
        let preparation = if matches!(reply, Reply::WrongThirdOperation) {
            CatalogOutput::NalBootstrapPlan {
                outcome: Box::new(bootstrap),
            }
        } else {
            CatalogOutput::NalAnnotationPreparation {
                outcome: Box::new(NalAnnotationPreparation {
                    bootstrap,
                    decision: NalAnnotationDecision::Existing {
                        annotation: annotation(),
                    },
                }),
            }
        };
        let validation = CatalogOutput::NalAnnotationValidation {
            outcome: if matches!(reply, Reply::Invalid) {
                NalAnnotationValidation::InvalidRequest {}
            } else {
                NalAnnotationValidation::Valid {}
            },
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
        let mut script = HOST.replace(
            "__INVALID__",
            if matches!(reply, Reply::Invalid) {
                "yes"
            } else {
                "no"
            },
        );
        for (marker, output) in [
            ("__VALIDATION__", validation),
            ("__PROBES__", probes),
            ("__PREPARATION__", preparation),
        ] {
            let output = serde_json::to_string(&output).unwrap();
            assert!(!output.contains('\''), "fixed synthetic shell literals");
            script = script.replace(marker, &output);
        }
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let manifest_file = root.join("manifest.json");
        let mut manifest: serde_json::Value =
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
                    (name.starts_with("nal-bootstrap-") || name.starts_with("nal-annotation-"))
                        && name.ends_with(".v1.sql")
                })
            })
            .collect::<Vec<_>>();
        resources.sort();
        assert_eq!(
            resources.len(),
            48,
            "45 startup and three annotation resources"
        );
        for name in resources {
            fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let tx = sql.transaction().unwrap();
        let page = crate::notes::create_page_with_paper_in(
            &tx,
            crate::notes::NotePageDraft {
                title: "Existing lifecycle note".into(),
                parent_page_id: None,
                parent_block_position: None,
            },
            None,
            Some(PAGE),
        )
        .unwrap();
        assert_eq!(page, PAGE);
        tx.execute(
            "insert into nal_statements(id,subject,predicate,origin) values(?1,?2,?3,'human')",
            params![TARGET, SUBJECT, PREDICATE],
        )
        .unwrap();
        tx.execute(
            "insert into nal_annotations(id,statement_id,note_page_id) values(?1,?2,?3)",
            params![ANNOTATION, TARGET, PAGE],
        )
        .unwrap();
        // Explicit test-only start revision, not a fresh migration default.
        tx.execute("update nal_state set legacy_imported=0,relation_semantics_version=0,revision=0 where id=1", []).unwrap();
        tx.commit().unwrap();
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

    pub fn snapshot(&self) -> Snapshot {
        snapshot(&self.db.lock().unwrap())
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.input_directory.exists());
        assert!(!self.execution_directory.exists());
        assert!(!self.executable.exists());
    }
}

pub(super) fn request() -> NalAnnotationRequest {
    NalAnnotationRequest {
        target_kind: "statement".into(),
        target_id: TARGET.into(),
        title: "Existing lifecycle note".into(),
    }
}

pub(super) fn annotation() -> NalAnnotation {
    NalAnnotation {
        id: ANNOTATION.into(),
        target_kind: "statement".into(),
        target_id: TARGET.into(),
        note_page_id: PAGE.into(),
    }
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
                statements: vec![RawNalStatementV1 {
                    id: TARGET.into(),
                    subject_json: SUBJECT.into(),
                    predicate_json: PREDICATE.into(),
                    origin: "human".into(),
                }],
                annotations: vec![annotation()],
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
    pub notes: Vec<NoteSnapshot>,
    pub annotations: Vec<AnnotationSnapshot>,
    pub total_changes: u64,
}

#[derive(Debug, PartialEq)]
pub(super) struct NoteSnapshot {
    page_id: String,
    title: String,
    status: String,
    revision: i64,
    block_id: String,
    block_type: String,
    props_json: String,
    content_json: String,
    plain_text: String,
}

#[derive(Debug, PartialEq)]
pub(super) struct AnnotationSnapshot {
    id: String,
    concept_id: Option<String>,
    statement_id: Option<String>,
    note_page_id: String,
}

pub(super) fn snapshot(db: &Connection) -> Snapshot {
    let flags = db
        .query_row(
            "select legacy_imported,relation_semantics_version,revision from nal_state where id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let notes = db.prepare("select p.id,p.title,p.status,p.revision,b.id,b.block_type,b.props_json,b.content_json,b.plain_text from note_pages p join note_blocks b on b.page_id=p.id where p.id=?1 order by b.id").unwrap()
        .query_map([PAGE], |r| Ok(NoteSnapshot { page_id:r.get(0)?,title:r.get(1)?,status:r.get(2)?,revision:r.get(3)?,block_id:r.get(4)?,block_type:r.get(5)?,props_json:r.get(6)?,content_json:r.get(7)?,plain_text:r.get(8)? })).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let annotations = db
        .prepare("select id,concept_id,statement_id,note_page_id from nal_annotations order by id")
        .unwrap()
        .query_map([], |r| {
            Ok(AnnotationSnapshot {
                id: r.get(0)?,
                concept_id: r.get(1)?,
                statement_id: r.get(2)?,
                note_page_id: r.get(3)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    Snapshot {
        flags,
        notes,
        annotations,
        total_changes: db.total_changes(),
    }
}
