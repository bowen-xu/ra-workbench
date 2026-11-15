//! One canned preparation frame; actual Core SQL/log and Note owners settle it.
use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use research_workbench_native_sdk::{catalog::CatalogOutput, reading_units::*};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};

const PAGE: &str = "synthetic-reading-unit-service-core-note";
const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
case "$line" in *'"operation":"prepareReadingUnit"'*) ;; *) exit 42 ;; esac
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__PREPARATION__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
IFS= read -r line
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 43 ;; esac
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

pub(super) struct Fixture {
    _package: tempfile::TempDir,
    data: tempfile::TempDir,
    pub db: SharedDatabase,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    input_directory: PathBuf,
    execution_directory: PathBuf,
}
impl Fixture {
    pub async fn new() -> Self {
        let output = serde_json::to_string(&CatalogOutput::ReadingUnitPreparation {
            outcome: Box::new(preparation()),
        })
        .unwrap();
        assert!(!output.contains('\''), "fixed synthetic shell literal");
        let (package, catalog, _) =
            literature_fixture(HOST.replace("__PREPARATION__", &output).as_bytes());
        let root = package.path().join("writing");
        let file = root.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        // Same immutable source copy as the Core adapter fixture; the actual
        // production loader still accepts only its original eight exact grants.
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "sql") {
                fs::copy(entry.path(), root.join("data").join(entry.file_name())).unwrap();
            }
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let tx = sql.transaction().unwrap();
        crate::notes::create_page_with_paper_in(
            &tx,
            crate::notes::NotePageDraft {
                title: "Synthetic concept lifecycle Core Note".into(),
                parent_page_id: None,
                parent_block_position: None,
            },
            None,
            Some(PAGE),
        )
        .unwrap();
        let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
        document.blocks[0].content = json!({"kind":"inline","items":[{
            "type":"text","text":"Core Note survives cancelled concept caller and host shutdown","styles":[]
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
        let db = Database::open(sql, &data.path().join("data-log")).unwrap();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let input_directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let execution_directory = staged.directory.path().to_owned();
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
    pub fn cold_snapshot(self) -> Snapshot {
        let Self {
            data, db, service, ..
        } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&db), 1);
        drop(service);
        db.shutdown().unwrap();
        drop(db);
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let reopened = Database::open(sql, &data.path().join("data-log")).unwrap();
        let state = snapshot(&reopened.lock().unwrap());
        reopened.shutdown().unwrap();
        state
    }
}
pub(super) fn request() -> ReadingUnitRequest {
    ReadingUnitRequest::Save {
        draft: Box::new(draft()),
    }
}
fn preparation() -> ReadingUnitPreparationV1 {
    plan(draft())
}
pub(super) fn draft() -> StructuredReadingUnitDraft {
    StructuredReadingUnitDraft {
        id: None,
        project_id: None,
        paper_id: None,
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
#[derive(Debug, PartialEq)]
pub(super) struct Snapshot {
    pub document: Value,
    pub rows: BTreeMap<&'static str, Vec<String>>,
}
fn snapshot(db: &Connection) -> Snapshot {
    let mut rows = BTreeMap::new();
    for (key, query) in [
        (
            "units",
            "select json_array(id,project_id,paper_id,unit_kind,title,body,provenance_kind,epistemic_role,revision,created_at,updated_at,reverted_at) from structured_reading_units order by id",
        ),
        (
            "revisions",
            "select json_array(id,object_kind,object_id,revision,snapshot_json,editor_kind,change_reason,created_at) from research_object_revisions order by id",
        ),
        (
            "evidence",
            "select json_array(id,reading_unit_id,anchor_id,stance,rationale,created_at) from reading_unit_evidence_links order by id",
        ),
    ] {
        rows.insert(
            key,
            db.prepare(query)
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap(),
        );
    }
    Snapshot {
        document: serde_json::to_value(crate::notes::load_document_value(db, PAGE).unwrap())
            .unwrap(),
        rows,
    }
}
