use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use research_workbench_native_sdk::nal_query::{SavePlan, StoredRow};
use rusqlite::Connection;
use serde_json::json;
use std::{fs, path::PathBuf};

// The original service fixture's fixed-frame/sidecar mechanism, with canned
// typed output only. This does not run or recreate the domain decision engine.
const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
case "$line" in *'"operation":"__OPERATION__"'*) ;; *) exit 42 ;; esac
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__OUTPUT__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
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
    pub content: Content,
}

impl Fixture {
    pub async fn prepare_save() -> Self {
        let content = content();
        let output = CatalogOutput::NalQuerySavePlan {
            outcome: PrepareOutcome::Planned {
                plan: SavePlan {
                    definitions_json: serde_json::to_string(&content.definitions).unwrap(),
                    cache_json: content
                        .cache
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .unwrap(),
                    legacy_imported: content.legacy_imported,
                },
            },
        };
        Self::new(output, "prepareNalQuerySave", content).await
    }

    pub async fn wrong_read() -> Self {
        let output = CatalogOutput::NalQueries {
            outcome: ProjectionOutcome::Success {
                row: StoredRow {
                    revision: 99,
                    definitions_json: " [] ".into(),
                    cache_json: Some("null".into()),
                    legacy_imported: true,
                },
            },
        };
        Self::new(output, "readNalQueries", content()).await
    }

    async fn new(output: CatalogOutput, operation: &str, content: Content) -> Self {
        let output = serde_json::to_string(&output).unwrap();
        assert!(!output.contains('\''), "one synthetic shell literal");
        let script = HOST
            .replace("__OUTPUT__", &output)
            .replace("__OPERATION__", operation);
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let manifest_path = root.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.nal-query-collections","version":1}));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for name in ["nal-query-read.v1.sql", "nal-query-save.v1.sql"] {
            fs::copy(source.join(name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
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
            content,
        }
    }

    pub fn snapshot(&self) -> (StoredRow, u64) {
        let db = self.db.lock().unwrap();
        let row = db.query_row(
            "select revision,definitions,cache,legacy_imported from nal_query_collections where id=1",
            [], |row| Ok(StoredRow { revision: row.get(0)?, definitions_json: row.get(1)?,
                cache_json: row.get(2)?, legacy_imported: row.get(3)? }),
        ).unwrap();
        (row, db.total_changes())
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.input_directory.exists());
        assert!(!self.execution_directory.exists());
        assert!(!self.executable.exists());
    }
}

fn content() -> Content {
    serde_json::from_value(
        json!({"definitions":[{"id":"synthetic-query","name":"Committed query",
        "conditions":[{"subject":{"kind":"variable","name":"x"},
            "predicate":{"kind":"concept","id":"synthetic-domain"}}],"outputVariables":["x"]}],
        "cache":{"version":0,"opaque":[false,null]},"legacyImported":true}),
    )
    .unwrap()
}
