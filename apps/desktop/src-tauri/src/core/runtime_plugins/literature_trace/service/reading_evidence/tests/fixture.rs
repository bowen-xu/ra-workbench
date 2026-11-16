use super::*;
use crate::{
    data_interface::{Database, SharedDatabase},
    runtime_plugins::native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

// This real host accepts only initialization and shutdown. A source lookup is
// Core SQL, so any synthetic Catalog RPC makes the fixture fail closed.
const HOST: &str = r#"#!/bin/sh
IFS= read -r line
printf 'initialize\n' >> lifecycle.log
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
case "$line" in *'"kind":"shutdown"'*) ;; *) printf 'unexpected\n' >> lifecycle.log; exit 43 ;; esac
printf 'shutdown\n' >> lifecycle.log
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

pub(super) fn package() -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String) {
    let (root, catalog, _) = literature_fixture(HOST.as_bytes());
    let directory = root.path().join("writing");
    let file = directory.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":crate::runtime_plugins::reading_evidence::CAPABILITY_ID,"version":1}));
    fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in [
        "reading-evidence-snapshot-hash.v1.sql",
        "reading-evidence-snapshot-rows.v1.sql",
    ] {
        fs::copy(source.join(name), directory.join("data").join(name)).unwrap();
    }
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    (root, catalog, key)
}
pub(super) struct Fixture {
    pub package: tempfile::TempDir,
    data: tempfile::TempDir,
    pub db: SharedDatabase,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    input: Option<PathBuf>,
    execution: Option<PathBuf>,
}
impl Fixture {
    pub async fn new(provider: bool) -> Self {
        let (package, catalog, key) = package();
        if !provider {
            fs::remove_dir_all(package.path().join("writing")).unwrap();
        }
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        sql.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Synthetic paper','now','now');insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('unit','paper','research_question','Question','Original manual body.','question','now','now');").unwrap();
        let db = Database::open(sql, &data.path().join("data-log")).unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        let (input, execution) = if provider {
            let staged =
                stage_for(&service.catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            let input = staged.input_directory.as_ref().unwrap().path().to_owned();
            let execution = staged.directory.path().to_owned();
            let mut host =
                NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
            timeout(WAIT, host.initialize()).await.unwrap().unwrap();
            *service.host.lock().await = Some(host);
            (Some(input), Some(execution))
        } else {
            (None, None)
        };
        Self {
            package,
            data,
            db,
            service,
            key,
            input,
            execution,
        }
    }
    pub fn legacy() -> LegacyImportDraft {
        LegacyImportDraft {
            request_id: "synthetic-legacy".into(),
            catalog: None,
            paper_id: Some("paper".into()),
            presentation: None,
        }
    }
    pub fn startup() -> StartupImportDraft {
        StartupImportDraft {
            catalog: None,
            presentations: vec![],
            contexts: vec![],
        }
    }
    pub fn imported(&self) -> i64 {
        self.db.lock().unwrap().query_row("select count(*) from structured_resource_legacy_imports where import_key='paper:paper'",[],|r|r.get(0)).unwrap()
    }
    pub fn body(&self) -> String {
        self.db.lock().unwrap().query_row("select content_json from structured_resources where paper_id='paper' and kind='structuredReading'",[],|r|r.get(0)).unwrap()
    }
    pub fn assert_no_catalog_rpc(&self) {
        if let Some(input) = self.input.as_ref() {
            assert_eq!(
                fs::read_to_string(input.join("lifecycle.log")).unwrap(),
                "initialize\n"
            );
        }
    }
    pub async fn assert_closed(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        for path in [&self.input, &self.execution].into_iter().flatten() {
            assert!(
                !path.exists(),
                "temporary host owner not cleaned: {}",
                path.display()
            );
        }
    }
    pub fn cold_reopen(self) -> (i64, String) {
        let templates = template_versions(&self.db.lock().unwrap());
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
        let guard = reopened.lock().unwrap();
        assert_eq!(template_versions(&guard), templates);
        let count=guard.query_row("select count(*) from structured_resource_legacy_imports where import_key='paper:paper'",[],|r|r.get(0)).unwrap();
        let body=guard.query_row("select coalesce(content_json,'') from structured_resources where paper_id='paper' and kind='structuredReading'",[],|r|r.get(0)).unwrap();
        drop(guard);
        reopened.shutdown().unwrap();
        (count, body)
    }
}

fn template_versions(db: &Connection) -> Vec<(String, i64, String, String)> {
    db.prepare("select legacy_id,legacy_revision,resource_id,payload_json from structured_resource_legacy_versions where legacy_id like 'reading-profile:%' or legacy_id like 'template:%' order by legacy_id,legacy_revision")
        .unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
}
