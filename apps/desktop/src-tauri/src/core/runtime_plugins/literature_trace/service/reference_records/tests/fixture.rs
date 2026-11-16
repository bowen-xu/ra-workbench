//! Real SQL/log/Note and managed stdio; only the protocol reply is canned data.
use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::{
        literature_duplicates::tests::fixture::{Fixture as CoreFixture, PAGE},
        native::package::{stage_for, tests::literature_fixture},
    },
};
use research_workbench_native_sdk::{
    literature_trace_protocol::Request,
    reference_records::{ReferenceRecordDraft, ReferenceRecordPlanV1},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

#[path = "gate.rs"]
mod gate;
use gate::Gate;

const HOST: &str = r#"#!/bin/sh
IFS= read -r line || exit 40
printf 'initialize\n' >> '__OBS__/calls'
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line || exit 41
case "$line" in
*'"kind":"shutdown"'*) ;;
*'"operation":"prepareReferenceRecord"'*)
printf 'prepare\n' >> '__OBS__/calls'
printf '%s' "$line" > '__OBS__/request.json'
if [ '__HOLD__' = yes ]; then
printf e > '__OBS__/entered'
IFS= read -r release < '__OBS__/release' || exit 42
fi
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__OUTPUT__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
if [ '__STOP__' = yes ]; then
# Keep this fixture's owned root alive for deterministic retirement.
kill -STOP "$$"
exit 43
fi
IFS= read -r line || exit 44 ;;
*) exit 45 ;;
esac
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 46 ;; esac
printf 'shutdown\n' >> '__OBS__/calls'
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Held,
    WrongDraft,
    WrongKind,
    Rejected,
    #[cfg(debug_assertions)]
    Overflow,
}

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub initial_identity: (String, String),
    pub draft: ReferenceRecordDraft,
    observations: tempfile::TempDir,
    gate: Gate,
    inputs: PathBuf,
    executable: PathBuf,
    sql_path: PathBuf,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub async fn new(mode: Mode) -> Self {
        let data = CoreFixture::new();
        let sql_path = PathBuf::from(data.db.lock().unwrap().path().unwrap());
        data.db
            .lock()
            .unwrap()
            .execute_batch(
                "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
             values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
             'aa/synthetic-reference.pdf','application/pdf',10,'before');
             insert into attachments(id,item_id,content_sha256,display_name,created_at)
             values('reference-service-attachment','survivor',
             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
             'synthetic-reference.pdf','before');
             insert into source_anchors
             (id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at)
             values('reference-service-anchor','survivor','reference-service-attachment',
             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
             0,'Synthetic reference quote','before');",
            )
            .unwrap();
        let mut draft: ReferenceRecordDraft = serde_json::from_value(json!({
            "citingPaperId":"survivor","sourceAnchorId":"reference-service-anchor",
            "rawText":" Synthetic reference body ","fields":[{
                "name":"title","value":"Synthetic cited title","sourceKind":"human",
                "confirmedByUser":true
            }],"parseState":"parsed","identityState":"unverified",
            "semanticState":"unreviewed","changeReason":" manual_reference_capture "
        }))
        .unwrap();
        if matches!(mode, Mode::Rejected) {
            draft.raw_text = " ".into();
        }
        #[cfg(debug_assertions)]
        if matches!(mode, Mode::Overflow) {
            let mut previous = draft.clone();
            previous.raw_text = "Preserved reference before overflow".into();
            let mut db = data.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            tx.execute(
                "insert into reference_records
                 (id,citing_paper_id,source_anchor_id,raw_text,fields_json,parse_state,
                  identity_state,semantic_state,usage_summary,provenance_kind,
                  current_revision,created_at,updated_at)
                 values('reference-overflow-record','survivor','reference-service-anchor',
                        ?1,?2,'parsed','unverified','unreviewed','','human_original',?3,
                        'before','before')",
                rusqlite::params![
                    previous.raw_text,
                    serde_json::to_string(&previous.fields).unwrap(),
                    i64::MAX
                ],
            )
            .unwrap();
            tx.execute(
                "insert into reference_record_revisions
                 (id,reference_record_id,revision,snapshot_json,editor_kind,change_reason,created_at)
                 values('reference-overflow-history','reference-overflow-record',?1,?2,
                        'human','preserved-before-overflow','before')",
                rusqlite::params![i64::MAX, serde_json::to_string(&previous).unwrap()],
            )
            .unwrap();
            tx.commit().unwrap();
            draft.id = Some("reference-overflow-record".into());
            draft.expected_revision = Some(i64::MAX);
        }
        // One fixed synthetic reply, not a copy of the Native domain planner.
        let mut echoed = draft.clone();
        if matches!(mode, Mode::WrongDraft) {
            echoed.source_anchor_id = "wrong-echo".into();
        }
        let output = match mode {
            Mode::WrongKind => CatalogOutput::MergeOperations { items: vec![] },
            Mode::Rejected => CatalogOutput::ReferenceRecordPreparation {
                outcome: Box::new(ReferenceRecordPreparationV1::Rejected {
                    reason: ReferenceRecordRejectionV1::InvalidFields,
                }),
            },
            _ => CatalogOutput::ReferenceRecordPreparation {
                outcome: Box::new(ReferenceRecordPreparationV1::Ready {
                    plan: Box::new(ReferenceRecordPlanV1 {
                        fields_json: serde_json::to_string(&echoed.fields).unwrap(),
                        snapshot_json: serde_json::to_string(&echoed).unwrap(),
                        draft: echoed,
                        raw_text: "Synthetic reference body".into(),
                        usage_summary: String::new(),
                        change_reason: "manual_reference_capture".into(),
                    }),
                }),
            },
        };
        let observations = tempfile::tempdir().unwrap();
        let gate = Gate::new(observations.path());
        let encoded = serde_json::to_string(&output).unwrap();
        let directory = observations.path().to_str().unwrap();
        assert!(!encoded.contains('\'') && !directory.contains('\''));
        let script = HOST
            .replace("__OBS__", directory)
            .replace("__OUTPUT__", &encoded)
            .replace(
                "__HOLD__",
                if matches!(mode, Mode::Held) {
                    "yes"
                } else {
                    "no"
                },
            )
            .replace(
                "__STOP__",
                if matches!(mode, Mode::WrongDraft | Mode::WrongKind) {
                    "yes"
                } else {
                    "no"
                },
            );
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":"literature.reference-records","version":1
        }));
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for key in [
            "ids",
            "record",
            "anchor-belongs",
            "matched-exists",
            "insert",
            "header",
            "update",
            "revision",
        ] {
            let name = format!("reference-records-{key}.v1.sql");
            fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages.remove(0).package_key;
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let inputs = staged.input_directory.as_ref().unwrap().path().to_owned();
        let executable = staged.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let initial_identity = host.request_identity().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            data,
            service,
            key,
            initial_identity,
            draft,
            observations,
            gate,
            inputs,
            executable,
            sql_path,
            _package: package,
        }
    }

    pub fn start(&self) -> tokio::task::JoinHandle<NativeResult<ReferenceResult>> {
        let service = Arc::clone(&self.service);
        let db = Arc::clone(&self.data.db);
        let key = self.key.clone();
        let draft = self.draft.clone();
        tokio::spawn(async move {
            service
                .reference_records(
                    db,
                    key,
                    ReferenceRequest::Save {
                        draft: Box::new(draft),
                    },
                )
                .await
        })
    }

    pub async fn entered(&self) {
        timeout(WAIT, self.gate.entered()).await.unwrap();
    }

    pub fn release(&self) {
        self.gate.release();
    }

    pub fn steps(&self) -> Vec<String> {
        fs::read_to_string(self.observations.path().join("calls"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn request(&self) -> Request {
        serde_json::from_slice(&fs::read(self.observations.path().join("request.json")).unwrap())
            .unwrap()
    }

    pub async fn cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.inputs.exists() && !self.executable.exists());
    }

    pub fn cold(self) -> CoreFixture {
        let Self { data, service, .. } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&data.db), 1);
        drop(service);
        data.reopen()
    }

    pub fn cold_fenced(self) -> Value {
        let Self {
            data,
            service,
            sql_path,
            ..
        } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&data.db), 1);
        drop(service);
        // Drop the old fenced owner, retaining its original TempDir lifetime.
        // No raw access through that owner and no successful shutdown is claimed.
        drop(data.db);
        let mut sql = Connection::open(&sql_path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &sql_path.parent().unwrap().join("data-log")).unwrap();
        let state = snapshot(&db);
        db.shutdown().unwrap();
        drop(db);
        state
    }
}

pub(super) fn save_note(database: &SharedDatabase) -> Value {
    let mut db = database.lock().unwrap();
    let tx = db.transaction().unwrap();
    let expected_revision = tx
        .query_row(
            "select revision from note_pages where id=?1",
            [PAGE],
            |row| row.get(0),
        )
        .unwrap();
    let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
    document.blocks[0].content = json!({"kind":"inline","items":[{
        "type":"text","text":"Core Note saved while reference preparation waits","styles":[]
    }]});
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    serde_json::to_value(crate::notes::load_document_value(&db, PAGE).unwrap()).unwrap()
}

pub(super) fn snapshot(db: &SharedDatabase) -> Value {
    let db = db.lock().unwrap();
    let rows = |sql| {
        db.prepare(sql)
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .into_iter()
            .map(|row| serde_json::from_str::<Value>(&row).unwrap())
            .collect::<Vec<_>>()
    };
    json!({
        "records":rows("select json_array(id,citing_paper_id,source_anchor_id,raw_text,fields_json,
             parse_state,identity_state,matched_item_id,semantic_state,usage_kind,usage_summary,
             usage_anchor_id,provenance_kind,current_revision,created_at,updated_at)
             from reference_records order by id"),
        "revisions":rows("select json_array(id,reference_record_id,revision,snapshot_json,
             editor_kind,change_reason,created_at) from reference_record_revisions order by id"),
        "anchors":rows("select json_array(id,item_id,attachment_id,content_sha256,page_index,quote_exact)
             from source_anchors order by id"),
        "note":serde_json::to_value(crate::notes::load_document_value(&db, PAGE).unwrap()).unwrap()
    })
}
