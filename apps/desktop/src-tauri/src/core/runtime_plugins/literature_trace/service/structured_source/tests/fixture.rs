use super::*;
use crate::{
    agent::{
        context_session::{resource_grants, sources::Resource},
        repository::submissions,
    },
    runtime_events::RuntimeEvents,
    runtime_plugins::{
        literature_context::PaperContextResources, literature_trace::binding::NoteTraceBinding,
    },
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    paper_context::*,
    paper_context_wire::{
        PaperContextDecisionV1 as Decision, PaperContextInputV1 as Input, PaperContextOutcomeV1,
        encode_payload,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;
const HOST: &str = r#"#!/bin/sh
IFS= read -r line
printf 'initialize\n' >> '__LOG__'
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
number=0
while IFS= read -r line; do
 identity=${line%\"body\":*}
 case "$line" in
  *'"kind":"shutdown"'*) printf 'shutdown\n' >> '__LOG__'; printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"; exit 0;;
  *'"operation":"preparePaperContext"'*|*'"operation":"decidePaperRetrievalTrace"'*) ;;
  *) exit 43;;
 esac
 request=${line#*\"requestId\":\"}; request=${request%%\"*}
 printf 'request %s\n' "$request" >> '__LOG__'
 printf 'call %s\n' "$number" >> '__LOG__'
 __GATE__
 __TRANSPORT__
 package=${line#*\"packageKey\":\"}; package=${package%%\"*}
 instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
 result=catalog-result-${request}.json
 case "$number" in
 __OUTPUTS__
 *) exit 44;;
 esac
 number=$((number+1))
 bytes=$(/usr/bin/wc -c < "$result")
 digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
 printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
done
"#;
pub(super) const WAIT: Duration = Duration::from_secs(10);
pub(super) struct Fixture {
    pub package: tempfile::TempDir,
    pub data: tempfile::TempDir,
    pub _control: tempfile::TempDir,
    pub store: AppStore,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub record: SubmissionRecord,
    pub call: ToolCall,
    pub log: PathBuf,
    pub release: PathBuf,
}
pub(super) struct Prepared {
    pub build: CatalogOutput,
    pub project: CatalogOutput,
    pub digest: CatalogOutput,
    pub projection: SnapshotProjectionV1,
}
fn outcome(input: Input, decision: Decision) -> CatalogOutput {
    CatalogOutput::PaperContext {
        outcome: Box::new(PaperContextOutcomeV1 {
            input: Box::new(input),
            decision,
        }),
    }
}
impl Fixture {
    pub fn new() -> Self {
        Self::with_sources(AppStoreSources::default())
    }
    fn with_sources(sources: AppStoreSources) -> Self {
        let control = tempfile::tempdir().unwrap();
        let log = control.path().join("calls.log");
        let release = control.path().join("release");
        let (package, catalog, _) =
            crate::runtime_plugins::native::package::tests::literature_fixture(
                b"#!/bin/sh\nexit 42\n",
            );
        let directory = package.path().join("writing");
        let manifest = directory.join("manifest.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["provides"].as_array_mut().unwrap().extend([
            json!({"id":"literature.paper-context","version":1}),
            json!({"id":"literature.paper-context-preview","version":1}),
            json!({"id":"literature.paper-context-prepare","version":1}),
            json!({"id":"literature.reading-evidence","version":1}),
            json!({"id":"literature.paper-retrieval-trace","version":1}),
        ]);
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        let names = [
            "attachments-paper-exists",
            "attachments-all",
            "attachments-exact",
            "snapshot-load-header",
            "snapshot-load-attachments",
            "snapshot-load-items",
            "snapshot-persist-find-hash",
            "snapshot-membership",
            "snapshot-item-geometry",
            "snapshot-repair-delete-items",
            "snapshot-repair-delete-attachments",
            "snapshot-repair-delete-header",
            "snapshot-persist-insert-header",
            "snapshot-persist-insert-attachment",
            "snapshot-persist-insert-item",
            "preview-header",
            "preview-items-budget",
            "preview-attachments-budget",
            "session-latest",
            "session-summary",
        ];
        for name in names {
            let file = format!("paper-context-{name}.v1.sql");
            fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
        }
        for name in ["snapshot-hash", "snapshot-rows"] {
            let file = format!("reading-evidence-{name}.v1.sql");
            fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        let data = tempfile::tempdir().unwrap();
        let store = open_with_sources(data.path(), sources);
        seed(&store);
        let record = submission(&store, None);
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_read_source".into(),
            arguments: json!({"fileId":"selected","view":"structured"}),
        };
        Self {
            package,
            data,
            _control: control,
            store,
            service,
            key,
            record,
            call,
            log,
            release,
        }
    }
    fn build_input(&self) -> PaperContextBuildInputV1 {
        let db = self.store.connection().unwrap();
        let package = self.service.catalog.discover().unwrap().packages.remove(0);
        let resources = PaperContextResources::load(&self.service.catalog, &package).unwrap();
        resources
            .capture_build(&db, "paper", &["selected".into()])
            .unwrap()
            .input
    }
    pub fn build_only(&self) -> CatalogOutput {
        build_response(&self.build_input()).0
    }
    pub fn prepared(&self) -> Prepared {
        let build = self.build_input();
        // Fixed public contract responses isolate Core ownership and settlement.
        // The domain package verifies the same captured inputs and outputs.
        let (built, mut prepared) = build_response(&build);
        let pages = crate::ai::authorized_pages(&[("selected".into(), Some(0))]).unwrap();
        crate::ai::restrict_items(&mut prepared.context_items, &pages);
        let input = SnapshotProjectionInputV1 {
            paper_id: "paper".into(),
            attachment_statuses: prepared.attachment_statuses,
            context_items: prepared.context_items,
            image_hashes: Default::default(),
        };
        let projection =
            crate::runtime_plugins::literature_context::test_fixture::project(&input).unwrap();
        let project = outcome(
            Input::Project {
                cbor_base64: encode_payload(&input).unwrap(),
            },
            Decision::Projected {
                projection_cbor_base64: encode_payload(&projection).unwrap(),
            },
        );
        let input = SnapshotDigestInputV1 {
            paper_id: "paper".into(),
            context_schema_version: projection.context_schema_version,
            attachments: projection.attachments.clone(),
            items: projection.items.clone(),
        };
        let digest =
            crate::runtime_plugins::literature_context::test_fixture::digest(&input).unwrap();
        Prepared {
            build: built,
            project,
            digest: outcome(
                Input::Digest {
                    cbor_base64: encode_payload(&input).unwrap(),
                },
                Decision::Digested {
                    context_hash: digest,
                },
            ),
            projection,
        }
    }
    pub fn configure(
        &mut self,
        outputs: &[CatalogOutput],
        gate: Option<usize>,
        transport_failure: bool,
    ) {
        assert!(self.service.host.try_lock().unwrap().is_none());
        let outputs = outputs
            .iter()
            .enumerate()
            .map(|(index, output)| {
                let value = serde_json::to_string(output).unwrap();
                assert!(!value.contains('\''));
                format!("{index}) printf '%s' '{value}' > \"$result\";;")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let gate=gate.map_or_else(String::new,|index|format!("if [ \"$number\" = {index} ]; then while [ ! -f '{}' ]; do /bin/sleep 0.01; done; fi",self.release.display()));
        let host = HOST
            .replace("__LOG__", self.log.to_str().unwrap())
            .replace("__GATE__", &gate)
            .replace(
                "__TRANSPORT__",
                if transport_failure { "exit 42" } else { ":" },
            )
            .replace("__OUTPUTS__", &outputs);
        fs::write(self.package.path().join("writing/host"), host).unwrap();
        self.key = self.service.catalog.discover().unwrap().packages[0]
            .package_key
            .clone();
        self.store = self.store.clone().with_native_note_trace(NoteTraceBinding {
            service: Arc::clone(&self.service),
            package_key: self.key.clone(),
        });
    }
    pub async fn read(&self) -> Result<Value, SourceReadError> {
        self.service
            .read_structured_source(
                self.store.clone(),
                self.key.clone(),
                self.record.clone(),
                self.call.clone(),
            )
            .await
    }
    pub fn spawn(&self) -> tokio::task::JoinHandle<Result<Value, SourceReadError>> {
        let service = Arc::clone(&self.service);
        let (store, key, record, call) = (
            self.store.clone(),
            self.key.clone(),
            self.record.clone(),
            self.call.clone(),
        );
        tokio::spawn(async move {
            service
                .read_structured_source(store, key, record, call)
                .await
        })
    }
    pub fn release(&self) {
        fs::write(&self.release, []).unwrap();
    }
    pub fn lifecycle(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
    pub async fn entered(&self, index: usize) -> bool {
        tokio::time::timeout(WAIT, async {
            loop {
                if self
                    .lifecycle()
                    .lines()
                    .any(|line| line == format!("call {index}"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok()
    }
    pub fn counts(&self) -> (i64, i64) {
        let db = self.store.connection().unwrap();
        counts(&db)
    }
    pub fn proof(&self) -> Value {
        let raw:String=self.store.connection().unwrap().query_row("select snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2",rusqlite::params![self.record.conversation_id.to_string(),self.record.request_id.to_string()],|row|row.get(0)).unwrap();
        serde_json::from_str(&raw).unwrap()
    }
    pub fn raw_parse(&self, value: &str) {
        self.store
            .connection()
            .unwrap()
            .execute(
                "update docling_parse_runs set json_content=?1 where id='parse-selected'",
                [value],
            )
            .unwrap();
    }
    pub fn snapshot_submission(&mut self, snapshot: &str) {
        self.record = submission(&self.store, Some(snapshot));
    }
}
fn build_response(build: &PaperContextBuildInputV1) -> (CatalogOutput, PreparedPaperContextV1) {
    let prepared = crate::runtime_plugins::literature_context::test_fixture::build(build);
    let output = outcome(
        Input::Build {
            cbor_base64: encode_payload(build).unwrap(),
        },
        Decision::Built {
            prepared_cbor_base64: encode_payload(&prepared).unwrap(),
        },
    );
    (output, prepared)
}
pub(super) fn open(path: &Path) -> AppStore {
    open_with_sources(path, AppStoreSources::default())
}
fn open_with_sources(path: &Path, sources: AppStoreSources) -> AppStore {
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: path.into(),
            agent_configuration_directory: path.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), sources),
    )
    .unwrap()
}
pub(super) fn counts(db: &rusqlite::Connection) -> (i64, i64) {
    (
        db.query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap(),
        db.query_row("select count(*) from agent_text_read_scopes", [], |row| {
            row.get(0)
        })
        .unwrap(),
    )
}
fn submission(store: &AppStore, snapshot: Option<&str>) -> SubmissionRecord {
    let mut db = store.connection().unwrap();
    let id = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title)values(?1,'Finite native reading')",
        [id.to_string()],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let mut context = resource_grants::grants(
        &tx,
        &[Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("selected".into()),
            page_index: Some(0),
        }],
        None,
    )
    .unwrap();
    if let Some(snapshot) = snapshot {
        let mut anchor: Value =
            serde_json::from_str(context[0].anchor.as_deref().unwrap()).unwrap();
        anchor["sourceSnapshotId"] = json!(snapshot);
        context[0].anchor = Some(anchor.to_string());
    }
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "Read only granted PDF page".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: ModelIdentity {
            provider_id: "fixture-generation".into(),
            model_id: "fixture-v1".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context,
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    };
    let record = submissions::insert_in(&tx, id, Uuid::new_v4(), input).unwrap();
    tx.commit().unwrap();
    record
}
fn seed(store: &AppStore) {
    let db = store.connection().unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at)values('paper','journalArticle','Core-only reading','now','now')",[]).unwrap();
    for (file, hash, text) in [
        ("selected", "a".repeat(64), "Authorized source"),
        ("unselected", "b".repeat(64), "Unshared secret"),
    ] {
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)values(?1,?2,'application/pdf',1,'now')",rusqlite::params![hash,format!("{file}.pdf")]).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at)values(?1,'paper',?2,?3,'now')",rusqlite::params![file,hash,format!("{file}.pdf")]).unwrap();
        let raw = json!({"pages":{"1":{"page_no":1,"size":{"width":200.0,"height":400.0}},"2":{"page_no":2,"size":{"width":200.0,"height":400.0}}},"texts":[{"self_ref":"#/texts/0","text":text,"prov":[{"page_no":1,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]},{"self_ref":"#/texts/1","text":"Ungranted page","prov":[{"page_no":2,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]}]});
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at)values(?1,'paper',?2,?3,'fixture','success','success',?4,'{}','now','now')",rusqlite::params![format!("parse-{file}"),file,hash,raw.to_string()]).unwrap();
    }
}
