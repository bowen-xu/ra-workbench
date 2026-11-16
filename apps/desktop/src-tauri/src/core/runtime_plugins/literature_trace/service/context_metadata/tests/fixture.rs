use super::*;
use crate::{
    agent::{
        context_session::{resource_grants, sources::Resource},
        source_tools::OwnedSubmissionTools,
    },
    runtime_plugins::{
        context_metadata::ContextMetadataResources, literature_trace::binding::NoteTraceBinding,
    },
    storage::AppStore,
};
use research_agent_core::{runner::ToolOutput, tool_registry::OwnedToolRegistration};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    context_metadata::{
        ContextMetadataDecisionV1, ContextMetadataOutcomeV1, ContextMetadataRequestV1,
    },
    paper_context_wire::encode_payload,
};
use std::{fs, path::PathBuf};
mod data;
pub(super) const WAIT: Duration = Duration::from_secs(10);
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
  *'"operation":"decideDerivedView"'*) ;;
  *'"operation":"validateEvidenceWrite"'*) ;;
  *'"operation":"validateEvidenceMatrix"'*) ;;
  *'"operation":"projectContextMetadata"'*) ;;
  *'"operation":"projectAnalysisHistory"'*) ;;
  *'"operation":"decideAnalysisWorkflow"'*) ;;
  *'"operation":"prepareBrowserCapture"'*) ;;
  *'"operation":"projectReadingHistory"'*) ;;
  *'"operation":"preparePaperContext"'*) ;;
  *'"operation":"preparePaperConfirmation"'*) ;;


  *'"operation":"prepareConceptRetrieval"'*) ;;
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

pub(super) struct Fixture {
    pub package: tempfile::TempDir,
    pub _data: tempfile::TempDir,
    pub _control: tempfile::TempDir,
    pub store: AppStore,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub record: SubmissionRecord,
    pub note: String,
    pub portrait: String,
    pub log: PathBuf,
    pub release_file: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        Self::for_paper("paper")
    }
    pub fn for_paper(paper: &str) -> Self {
        let control = tempfile::tempdir().unwrap();
        let log = control.path().join("calls.log");
        let release_file = control.path().join("release");
        let (package, catalog, _) =
            crate::runtime_plugins::native::package::tests::literature_fixture(
                b"#!/bin/sh\nexit 42\n",
            );
        let directory = package.path().join("writing");
        let manifest = directory.join("manifest.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.context-metadata","version":1}));
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for name in ["header", "creators", "identifiers"] {
            let file = format!("context-metadata-{name}.v1.sql");
            fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        let data = tempfile::tempdir().unwrap();
        let store = data::open(data.path());
        let (note, portrait) = data::seed(&store, paper);
        let record = data::submission(
            &store,
            vec![Resource::PaperMetadata {
                resource_id: paper.into(),
            }],
        );
        Self {
            package,
            _data: data,
            _control: control,
            store,
            service,
            key,
            record,
            note,
            portrait,
            log,
            release_file,
        }
    }
    pub fn mixed(&mut self, note_first: bool) {
        let note = Resource::Note {
            resource_id: uuid::Uuid::parse_str(&self.note).unwrap(),
        };
        let metadata = Resource::PaperMetadata {
            resource_id: "paper".into(),
        };
        let portrait = Resource::PaperPortrait {
            resource_id: "paper".into(),
        };
        self.record = data::submission(
            &self.store,
            if note_first {
                vec![note, metadata, portrait]
            } else {
                vec![metadata, note, portrait]
            },
        );
    }
    pub fn budget_notes(&mut self) {
        let second = data::budget_notes(&self.store, &self.note);
        self.record = data::submission(
            &self.store,
            vec![
                Resource::Note {
                    resource_id: uuid::Uuid::parse_str(&self.note).unwrap(),
                },
                Resource::Note {
                    resource_id: uuid::Uuid::parse_str(&second).unwrap(),
                },
                Resource::PaperMetadata {
                    resource_id: "paper".into(),
                },
                Resource::PaperPortrait {
                    resource_id: "paper".into(),
                },
            ],
        );
    }
    pub fn response(&self) -> CatalogOutput {
        let package = self.service.catalog.discover().unwrap().packages.remove(0);
        let resources = ContextMetadataResources::load(&self.service.catalog, &package).unwrap();
        let captured = resources
            .capture(&self.store.connection().unwrap(), "paper")
            .unwrap();
        assert_eq!(captured.input.paper_id, "paper");
        let mut body = captured.input.base.as_object().unwrap().clone();
        body.insert("creators".into(), json!(captured.input.creators));
        body.insert("identifiers".into(), json!(captured.input.identifiers));
        let body_json = serde_json::Value::Object(body).to_string();
        CatalogOutput::ContextMetadata {
            outcome: Box::new(ContextMetadataOutcomeV1 {
                input: Box::new(ContextMetadataRequestV1 {
                    cbor_base64: encode_payload(&captured.input).unwrap(),
                }),
                decision: ContextMetadataDecisionV1::Projected { body_json },
            }),
        }
    }
    pub fn configure(&mut self, outputs: &[CatalogOutput], gate: Option<usize>, fail: bool) {
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
        let gate = gate.map_or_else(String::new, |index| format!(
            "if [ \"$number\" = {index} ]; then while [ ! -f '{}' ]; do /bin/sleep 0.01; done; fi",
            self.release_file.display(),
        ));
        let host = HOST
            .replace("__LOG__", self.log.to_str().unwrap())
            .replace("__GATE__", &gate)
            .replace("__TRANSPORT__", if fail { "exit 42" } else { ":" })
            .replace("__OUTPUTS__", &outputs);
        fs::write(self.package.path().join("writing/host"), host).unwrap();
        self.bind();
    }
    pub fn bind(&mut self) {
        self.key = self.service.catalog.discover().unwrap().packages[0]
            .package_key
            .clone();
        self.store = self.store.clone().with_native_note_trace(NoteTraceBinding {
            service: Arc::clone(&self.service),
            package_key: self.key.clone(),
        });
    }
    pub fn bind_actual_package(
        &mut self,
        catalog: Arc<crate::runtime_plugins::RuntimePluginCatalog>,
        key: String,
    ) {
        assert!(self.service.host.try_lock().unwrap().is_none());
        assert!(self.store.paper_metadata_source().is_none());
        self.service = Arc::new(LiteratureTraceService::new(catalog));
        self.key = key;
        self.store = self.store.clone().with_native_note_trace(NoteTraceBinding {
            service: Arc::clone(&self.service),
            package_key: self.key.clone(),
        });
    }
    pub fn executor(&self) -> Arc<dyn ToolExecutor> {
        let frozen = OwnedSubmissionTools::new(self.record.clone());
        crate::agent::context_session::resource_tools::registration(
            self.store.clone(),
            self.record.clone(),
            OwnedToolRegistration::new(frozen.definitions(), Arc::new(frozen)),
        )
        .into_parts()
        .1
    }
    pub async fn execute(&self, call: ToolCall) -> ToolOutput {
        self.executor().authorize_and_execute(call).await.unwrap()
    }
    pub fn spawn(&self, call: ToolCall) -> tokio::task::JoinHandle<ToolOutput> {
        let executor = self.executor();
        tokio::spawn(async move { executor.authorize_and_execute(call).await.unwrap() })
    }
    pub fn metadata_read(&self) -> ToolCall {
        let source = self
            .record
            .input
            .context
            .iter()
            .find(|source| {
                matches!(
                    resource_grants::resource(source),
                    Some(Resource::PaperMetadata { .. })
                )
            })
            .unwrap();
        call("research_read_source", json!({"sourceId":source.source_id}))
    }
    pub fn search(&self) -> ToolCall {
        call("research_search_sources", json!({"query":"Ω"}))
    }
    pub fn change_note(&self, body: &str) {
        let content = json!({"kind":"inline","items":[{"type":"text","text":body,"styles":[]}]});
        self.store
            .connection()
            .unwrap()
            .execute(
                "update note_blocks set content_json=?1,plain_text=?2 where page_id=?3",
                rusqlite::params![content.to_string(), body, self.note],
            )
            .unwrap();
    }
    pub fn lifecycle(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
    pub fn calls(&self) -> usize {
        self.lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count()
    }
    pub fn release(&self) {
        fs::write(&self.release_file, []).unwrap();
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
    pub fn proofs(&self) -> i64 {
        self.store.connection().unwrap().query_row(
            "select count(*) from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2",
            rusqlite::params![self.record.conversation_id.to_string(),self.record.request_id.to_string()],
            |row| row.get(0),
        ).unwrap()
    }
}
pub(super) fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments,
    }
}
