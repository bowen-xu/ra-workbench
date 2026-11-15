use super::*;
use crate::runtime_plugins::{
    literature_duplicates::tests::fixture::Fixture as CoreFixture,
    native::package::{stage_for, tests::literature_fixture},
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput, catalog_save::CatalogDraft, import_jobs::ImportJobsProjectionOutcomeV1,
};
use serde_json::json;
use std::{fs, path::PathBuf};

pub(super) const SAVED_ID: &str = "service-import-job-item";

const HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
__PREPARE__
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__OUTPUT__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
__FINISH__
"#;

const PREPARE: &str = r#"case "$line" in *'"operation":"prepareBibtex"'*) ;; *) exit 42 ;; esac
: > prepare-seen
while [ ! -f release-prepare ]; do /bin/sleep 0.01; done"#;
const EXIT_PROJECT: &str = r#"IFS= read -r line
case "$line" in *'"operation":"projectImportJobs"'*) exit 41 ;; *) exit 42 ;; esac"#;

const SHUTDOWN: &str = r#"IFS= read -r line || exit 43
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 43 ;; esac
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub directory: PathBuf,
    pub executable: PathBuf,
    pub draft: CatalogDraft,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub async fn new(bibtex: bool) -> Self {
        let draft: CatalogDraft = serde_json::from_value(json!({
            "id":SAVED_ID,"itemType":"document","title":" Synthetic imported title ",
            "creators":[{"familyName":"Synthetic","givenName":"Author"}],
            "tags":["synthetic"],"sourceKind":"original-raw-source"
        }))
        .unwrap();
        // One fixed synthetic SDK reply: this fixture is not a domain planner.
        // Preserve the original draft JSON only as the stored raw-input payload.
        let output = if bibtex {
            CatalogOutput::BibtexPreparation {
                outcome: serde_json::from_value(json!({
                    "status":"ready","plan":{
                        "entries":[{"inputLabel":" Synthetic imported title ",
                            "rawInput":serde_json::to_string(&draft).unwrap(),
                            "save":{"kind":"planned","plan":{
                                "item":{"id":SAVED_ID,"itemType":"document",
                                    "title":"Synthetic imported title","abstractText":"",
                                    "issuedYear":null,"containerTitle":"","conferenceName":"",
                                    "proceedingsTitle":"","place":"","volume":"","issue":"",
                                    "pages":"","url":"","language":"","bibtexKey":"","rawBibtex":null},
                                "creators":[{"familyName":"Synthetic","givenName":"Author",
                                    "literalName":"","role":"author"}],
                                "doi":null,"tags":["synthetic"],
                                "provenance":{"fields":["itemType","title","abstractText","issuedYear",
                                    "containerTitle","conferenceName","proceedingsTitle","place","volume",
                                    "issue","pages","doi","url","language","bibtexKey","creators","tags"],
                                    "sourceKind":"bibtex","confirmedByUser":false},
                                "duplicates":{"normalizedTitle":"synthetic imported title","doiReason":"doi",
                                    "doiConfidence":1.0,"titleReason":"normalized_title_year","titleConfidence":0.72}
                            }}}],
                        "rules":{"sourceKind":"bibtex","importedState":"imported","failedState":"failed",
                            "finishStates":{"noFailed":"completed","noImported":"failed","mixed":"completed_with_errors"},
                            "notFoundMessage":"找不到文献或附件","emptyTitleMessage":"标题不能为空",
                            "unsupportedTypePrefix":"不支持的文献类型：","databaseErrorPrefix":"文献数据库操作失败："}
                    }
                })).unwrap(),
            }
        } else {
            CatalogOutput::ImportJobsProjection {
                outcome: ImportJobsProjectionOutcomeV1::NotFound {
                    message: "找不到文献或附件".into(),
                },
            }
        };
        let output = serde_json::to_string(&output).unwrap();
        assert!(!output.contains('\''), "fixed synthetic shell literal");
        let script = HOST
            .replace("__OUTPUT__", &output)
            .replace(
                "__PREPARE__",
                if bibtex {
                    PREPARE
                } else {
                    "case \"$line\" in *'\"operation\":\"projectImportJobs\"'*) ;; *) exit 42 ;; esac"
                },
            )
            .replace("__FINISH__", if bibtex { EXIT_PROJECT } else { SHUTDOWN });
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        for id in [
            "literature.import-jobs",
            "literature.catalog-read",
            "literature.catalog-save",
        ] {
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
            .join("../../../plugins/literature_library/native/data");
        for (prefix, names) in [
            (
                "import-jobs",
                &[
                    "header",
                    "results",
                    "create-header",
                    "insert-pending",
                    "counts",
                    "finish",
                    "result",
                    "list-ids",
                    "cancel",
                ][..],
            ),
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
        let key = catalog.discover().unwrap().packages.remove(0).package_key;
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let executable = staged.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            data: CoreFixture::new(),
            service,
            key,
            directory,
            executable,
            draft,
            _package: package,
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
