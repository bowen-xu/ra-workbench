use super::*;
use crate::{
    runtime_events::RuntimeEvents,
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_workbench_native_sdk::{
    candidate_contracts::CandidateValidationError, structured_validation::StructuredTargetV1,
};
use serde_json::json;
use std::{fs, path::Path};

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
  *'"operation":"validateStructured"'*|*'"operation":"selectUnreviewedFields"'*|*'"operation":"validateEditableTemplate"'*) ;;
  *) exit 43;;
 esac
 printf 'call %s\n' "$number" >> '__LOG__'
 package=${line#*\"packageKey\":\"}; package=${package%%\"*}
 instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
 request=${line#*\"requestId\":\"}; request=${request%%\"*}
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

pub(super) fn portrait(index: usize) -> Value {
    json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":format!("Synthetic coverage {index}"),
        "claims":(["genre","mode","paradigm"].into_iter().enumerate().map(|(i,dimension)|json!({"id":format!("claim_{i}"),"dimension":dimension,"label":"Uncertain","description":"Original synthetic description.","certainty":"uncertain","evidence":[]})).collect::<Vec<_>>()),
        "template":[{"id":"model","title":"Model","question":"What model?","claimIds":["claim_1"]}]})
}
pub(super) fn checkpoint(values: &[Value]) -> Value {
    json!({"format":1,"kind":"structured","content":values[0],"ancestor":values[1],"baselineAncestor":values[2],
        "observed":{"version":0,"saved":values[3],"baseline":values[4]},"reviewAncestor":values[5],"ordinary":[]})
}
pub(super) fn input(value: &Value) -> StructuredValidationInputV1 {
    StructuredValidationInputV1 {
        target: StructuredTargetV1::Portrait,
        candidate_json: value.to_string(),
        evidence: vec![],
    }
}
pub(super) struct Fixture {
    pub _log_root: tempfile::TempDir,
    pub _package: tempfile::TempDir,
    pub _data: tempfile::TempDir,
    pub store: AppStore,
    pub service: Arc<LiteratureTraceService>,
    pub id: String,
    pub key: String,
    pub log: std::path::PathBuf,
}
impl Fixture {
    pub fn new(values: &[Value], rejected: bool, events: RuntimeEvents) -> Self {
        // These canned protocol frames test the real owner/binding/transaction
        // path. Canonical policy correctness has its separate old/new oracle.
        let outputs = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let outcome = if rejected {
                    StructuredValidationOutcomeV1::Rejected {
                        input: Box::new(input(value)),
                        error: CandidateValidationError::Validation {
                            path: "$.claims[0]".into(),
                            message: "Original synthetic policy rejection".into(),
                        },
                    }
                } else {
                    StructuredValidationOutcomeV1::Validated {
                        input: Box::new(input(value)),
                        canonical_json: value.to_string(),
                    }
                };
                let output = serde_json::to_string(&CatalogOutput::StructuredValidation {
                    outcome: Box::new(outcome),
                })
                .unwrap();
                assert!(!output.contains('\''));
                format!("{index}) printf '%s' '{output}' > \"$result\";;")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("lifecycle.log");
        let host = HOST
            .replace("__LOG__", log.to_str().unwrap())
            .replace("__OUTPUTS__", &outputs);
        let (package, catalog, _) =
            crate::runtime_plugins::native::package::tests::literature_fixture(host.as_bytes());
        let directory = package.path().join("writing");
        let manifest = directory.join("manifest.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["provides"].as_array_mut().unwrap().extend([
            json!({"id":"literature.structured-validation","version":1}),
            json!({"id":"literature.reading-evidence","version":1}),
        ]);
        fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        for name in ["snapshot-hash", "snapshot-rows", "validation-latest"] {
            let name = format!("reading-evidence-{name}.v1.sql");
            fs::copy(source.join(&name), directory.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let store = AppStore::open_with_paths(
            AppStorePaths {
                data_directory: data.path().to_owned(),
                agent_configuration_directory: data.path().join("configuration"),
            },
            AppStorePorts::with_events(events, AppStoreSources::default()),
        )
        .unwrap();
        let id = {
            let mut db = store.connection().unwrap();
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Synthetic HBS service','now','now')",[]).unwrap();
            let reading = selection::select(&catalog, Some(&key)).unwrap();
            crate::structured_resources::import_startup_with_evidence(
                Some(&reading),
                &mut db,
                crate::structured_resources::StartupImportDraft {
                    catalog: None,
                    presentations: vec![],
                    contexts: vec![],
                },
                None,
            )
            .unwrap();
            crate::structured_resources::find_by_paper(&db, "paper", "portrait")
                .unwrap()
                .id
        };
        let service = Arc::new(LiteratureTraceService::new(catalog));
        Self {
            _log_root: root,
            _package: package,
            _data: data,
            store,
            service,
            id,
            key,
            log,
        }
    }
    pub fn lifecycle(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
    pub(super) fn responses(&mut self, values: &[CatalogOutput]) {
        assert!(self.service.host.try_lock().unwrap().is_none());
        let outputs = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let output = serde_json::to_string(value).unwrap();
                assert!(!output.contains('\''));
                format!("{index}) printf '%s' '{output}' > \"$result\";;")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let directory = self._package.path().join("writing");
        fs::write(
            directory.join("host"),
            HOST.replace("__LOG__", self.log.to_str().unwrap())
                .replace("__OUTPUTS__", &outputs),
        )
        .unwrap();
        if values
            .iter()
            .any(|value| matches!(value, CatalogOutput::UnreviewedFields { .. }))
        {
            let file = directory.join("manifest.json");
            let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            manifest["provides"].as_array_mut().unwrap().push(json!({
                "id":research_workbench_native_sdk::review_selection::CAPABILITY_ID,"version":1
            }));
            fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
        if values
            .iter()
            .any(|value| matches!(value, CatalogOutput::EditableTemplate { .. }))
        {
            let file = directory.join("manifest.json");
            let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            manifest["provides"].as_array_mut().unwrap().push(json!({
                "id":research_workbench_native_sdk::editable_template::CAPABILITY_ID,"version":1
            }));
            fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
        self.key = self.service.catalog.discover().unwrap().packages[0]
            .package_key
            .clone();
    }
}
