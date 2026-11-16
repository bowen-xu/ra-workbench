use super::*;
use crate::{
    data_interface::Database,
    runtime_plugins::native::package::{stage_for, tests::literature_fixture},
};
use rusqlite::Connection;
use serde_json::json;
use std::{fs, path::PathBuf};

const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
case "$line" in *'"operation":"prepareSave"'*) ;; *) exit 42 ;; esac
: > prepare-seen
while [ ! -f release-prepare ]; do /bin/sleep 0.01; done
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__OUTPUT__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
IFS= read -r line
case "$line" in *'"operation":"project"'*) exit 41 ;; *) exit 42 ;; esac
"#;

pub(super) struct Fixture {
    pub _package: tempfile::TempDir,
    pub data: tempfile::TempDir,
    pub db: SharedDatabase,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub directory: PathBuf,
    pub executable: PathBuf,
    pub draft: CatalogDraft,
}

impl Fixture {
    pub async fn new(id: Option<&str>) -> Self {
        // Canned valid domain output: this fixture does not implement a planner.
        let outcome = serde_json::from_value::<CatalogSaveOutcome>(json!({
            "kind":"planned","plan":{
                "item":{"id":id,"itemType":"journalArticle","title":"Committed synthetic title",
                    "abstractText":"Synthetic abstract","issuedYear":2026,"containerTitle":"Journal",
                    "conferenceName":"","proceedingsTitle":"","place":"","volume":"","issue":"",
                    "pages":"","url":"","language":"","bibtexKey":"synthetic","rawBibtex":null},
                "creators":[{"familyName":"Example","givenName":"Ada","literalName":"","role":"author"}],
                "doi":null,"tags":["synthetic"],
                "provenance":{"fields":["itemType","title","abstractText","issuedYear","containerTitle",
                    "conferenceName","proceedingsTitle","place","volume","issue","pages","doi","url",
                    "language","bibtexKey","creators","tags"],"sourceKind":"user","confirmedByUser":true},
                "duplicates":{"normalizedTitle":"committed synthetic title","doiReason":"doi",
                    "doiConfidence":1.0,"titleReason":"normalized_title_year","titleConfidence":0.72}
            }
        })).unwrap();
        let output = serde_json::to_string(&CatalogOutput::SavePlan { outcome }).unwrap();
        assert!(!output.contains('\''), "one synthetic shell literal");
        let (package, catalog, _) =
            literature_fixture(HOST.replace("__OUTPUT__", &output).as_bytes());
        let root = package.path().join("writing");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        for id in ["literature.catalog-read", "literature.catalog-save"] {
            manifest["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"version":1}));
        }
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for (prefix, names) in [
            (
                "catalog",
                &[
                    "list-active-ids",
                    "search-fts-ids",
                    "search-year-ids",
                    "item-main",
                    "item-creators",
                    "item-tags",
                    "item-attachments",
                    "item-project-ids",
                    "item-metadata-sources",
                ][..],
            ),
            (
                "catalog-save",
                &[
                    "item-upsert",
                    "creators-delete",
                    "creator-insert",
                    "item-creator-insert",
                    "doi-delete",
                    "doi-insert",
                    "tags-delete",
                    "tag-insert",
                    "item-tag-insert",
                    "provenance-upsert",
                    "search-delete",
                    "search-insert",
                    "duplicate-doi-ids",
                    "duplicate-title-ids",
                    "duplicate-insert",
                ][..],
            ),
        ] {
            for name in names {
                let name = format!("{prefix}-{name}.v1.sql");
                fs::copy(source.join(&name), root.join("data").join(name)).unwrap();
            }
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(data.path().join("store.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &data.path().join("data-log")).unwrap();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let executable = staged.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        let draft = serde_json::from_value(json!({
            "id":id,"itemType":"journalArticle","title":"  Committed synthetic title  ",
            "abstractText":" Synthetic abstract ","issuedYear":2026,"containerTitle":" Journal ",
            "bibtexKey":"synthetic","creators":[{"familyName":"Example","givenName":"Ada"}],
            "tags":["synthetic"],"sourceKind":"user"
        }))
        .unwrap();
        Self {
            _package: package,
            data,
            db,
            service,
            key,
            directory,
            executable,
            draft,
        }
    }

    pub async fn prepare_seen(&self) {
        timeout(WAIT, async {
            while !self.directory.join("prepare-seen").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    pub fn release_prepare(&self) {
        fs::write(self.directory.join("release-prepare"), []).unwrap();
    }
}
