//! Real Core SQL/log/Note and owned stdio; replies are typed canned test data.
use super::*;
use crate::runtime_plugins::{
    literature_duplicates::tests::fixture::{Fixture as CoreFixture, PAGE},
    native::package::{stage_for, tests::literature_fixture},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    data::DataValue,
    doi_metadata::{DoiPreviewInputV1, DoiPreviewOutcomeV1},
    literature_trace_protocol::Request,
    metadata::MetadataFieldDiff,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

mod gate;
mod host;
use gate::Gate;

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub instance: String,
    pub job_id: String,
    pub result_id: String,
    storage: DoiStorage,
    observations: tempfile::TempDir,
    inputs: PathBuf,
    executable: PathBuf,
    gate: Gate,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub async fn new(wrong_echo: bool) -> Self {
        let data = CoreFixture::new();
        let observations = tempfile::tempdir().unwrap();
        let gate = Gate::new(observations.path());
        let directory = observations.path().to_str().unwrap();
        assert!(!directory.contains('\''));
        let script = host::SCRIPT
            .replace("__OBS__", directory)
            .replace("__STOP__", if wrong_echo { "yes" } else { "no" });
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let directory = package.path().join("writing");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        for id in [
            "literature.catalog-read",
            "literature.import-jobs",
            "literature.doi-preview",
        ] {
            manifest["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"version":1}));
        }
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        copy_resources(&directory);
        let selected = catalog.discover().unwrap().packages.remove(0);
        let key = selected.package_key.clone();
        let storage = DoiStorage {
            database: Arc::clone(&data.db),
            resources: DoiResources::load(&catalog, &selected).unwrap(),
            jobs: ImportJobsResources::load(&catalog, &selected).unwrap(),
            catalog: CatalogResources::load(&catalog, &selected).unwrap(),
        };
        let (job_id, result_id) = storage::create_job(
            &mut data.db.lock().unwrap(),
            &storage.jobs,
            "doi",
            "10.1000/synthetic",
            " Raw DOI input ",
        )
        .unwrap();
        data.db.lock().unwrap().execute_batch(
            "insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at)
             values('survivor','title','doi.org:csl-json',0,'2026');
             create table synthetic_doi_calls(phase text not null);
             create trigger synthetic_doi_candidate after insert on metadata_candidates
             begin insert into synthetic_doi_calls values('candidate'); end;
             create trigger synthetic_doi_queued after update of state on import_jobs
             when new.state='queued'
             begin insert into synthetic_doi_calls values('queued'); end;"
        ).unwrap();
        let stage = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let inputs = stage.input_directory.as_ref().unwrap().path().to_owned();
        let executable = stage.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(stage, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let instance = host.request_identity().unwrap().0;
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            data,
            service,
            key,
            instance,
            job_id,
            result_id,
            storage,
            observations,
            inputs,
            executable,
            gate,
            _package: package,
        }
    }

    pub fn start(&self) -> tokio::task::JoinHandle<NativeResult<MetadataPreviewV1>> {
        let input = completion::CompleteDoi {
            storage: self.storage.clone(),
            key: self.key.clone(),
            instance: self.instance.clone(),
            normalized: "10.1000/synthetic".into(),
            item_id: Some("survivor".into()),
            translation: DoiTranslationV1 {
                draft: serde_json::from_value(json!({"itemType":"journalArticle",
                    "title":"Synthetic DOI candidate","doi":"10.1000/synthetic",
                    "sourceKind":"doi.org:csl-json"}))
                .unwrap(),
                raw_payload: "{\"synthetic\":true}".into(),
                source_kind: "doi.org:csl-json".into(),
                warnings: vec!["Synthetic warning".into()],
            },
            job_id: self.job_id.clone(),
            result_id: self.result_id.clone(),
        };
        let service = Arc::clone(&self.service);
        tokio::spawn(async move { service.complete_doi(input).await })
    }

    pub async fn entered(&self, index: usize) -> (Request, CatalogInput) {
        timeout(WAIT, self.gate.entered()).await.unwrap();
        let read = |name| fs::read(self.observations.path().join(name)).unwrap();
        let request: Request =
            serde_json::from_slice(&read(format!("request-{index}.json"))).unwrap();
        assert_eq!(request.package_key, self.key);
        assert_eq!(request.instance_id, self.instance);
        let input = serde_json::from_slice(&read(format!("input-{index}.json"))).unwrap();
        (request, input)
    }

    /// A canned echo, not an implementation of Native projection policy.
    pub fn release(
        &self,
        index: usize,
        input: &CatalogInput,
        wrong_echo: bool,
    ) -> MetadataPreviewV1 {
        let CatalogInput::DoiPreview { input } = input else {
            panic!("not DOI")
        };
        let DoiPreviewInputV1::Candidate {
            id,
            import_job_id,
            item_id,
            normalized_doi,
            translation,
            current,
            ..
        } = input.as_ref()
        else {
            panic!("not Candidate")
        };
        let current = current.as_ref().unwrap();
        let DataValue::Text(title) = &current.main.rows[0][2] else {
            panic!("not title")
        };
        let sources = &current.related.as_ref().unwrap().metadata_sources.rows;
        assert_eq!(sources.len(), 1);
        let mut draft = translation.draft.clone();
        draft.id.clone_from(item_id);
        let preview = MetadataPreviewV1 {
            id: if wrong_echo {
                "wrong-candidate-echo".into()
            } else {
                id.clone()
            },
            import_job_id: import_job_id.clone(),
            item_id: item_id.clone(),
            source_kind: translation.source_kind.clone(),
            source_reference: normalized_doi.clone(),
            draft,
            diffs: vec![MetadataFieldDiff {
                field_name: "title".into(),
                current_value: title.clone(),
                candidate_value: translation.draft.title.clone(),
                user_confirmed: sources[0][2] == DataValue::Integer(1),
            }],
            warnings: translation.warnings.clone(),
        };
        let output = CatalogOutput::DoiPreview {
            outcome: Box::new(DoiPreviewOutcomeV1::Candidate {
                preview: preview.clone(),
                raw_payload: translation.raw_payload.clone(),
            }),
        };
        fs::write(
            self.observations.path().join(format!("reply-{index}.json")),
            serde_json::to_vec(&output).unwrap(),
        )
        .unwrap();
        self.gate.release();
        preview
    }

    pub fn counts(&self) -> (i64, i64, i64) {
        self.data
            .db
            .lock()
            .unwrap()
            .query_row(
                "select (select count(*) from metadata_candidates),
             (select count(*) from synthetic_doi_calls where phase='candidate'),
             (select count(*) from synthetic_doi_calls where phase='queued')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    pub fn steps(&self) -> Vec<String> {
        fs::read_to_string(self.observations.path().join("calls"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub async fn save_note(&self) -> Value {
        let database = Arc::clone(&self.data.db);
        timeout(
            WAIT,
            tokio::task::spawn_blocking(move || {
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
                document.blocks[0].content = json!({"kind":"inline","items":[{"type":"text",
                "text":"Saved while DOI Candidate waits","styles":[]}]});
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
            }),
        )
        .await
        .unwrap()
        .unwrap()
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(
            !self
                .service
                .cleanup_failed
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(!self.inputs.exists());
        assert!(!self.executable.exists());
    }

    pub fn cold(self) -> CoreFixture {
        let Self {
            data,
            storage,
            service,
            ..
        } = self;
        drop(storage);
        assert_eq!(Arc::strong_count(&service), 1);
        drop(service);
        assert_eq!(Arc::strong_count(&data.db), 1);
        data.reopen()
    }
}

fn copy_resources(directory: &std::path::Path) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in [
        "doi-preview-result-failed",
        "doi-preview-candidate-insert",
        "doi-preview-job-queued",
        "import-jobs-header",
        "import-jobs-results",
        "import-jobs-create-header",
        "import-jobs-insert-pending",
        "import-jobs-counts",
        "import-jobs-finish",
        "import-jobs-result",
        "import-jobs-list-ids",
        "import-jobs-cancel",
        "catalog-list-active-ids",
        "catalog-search-fts-ids",
        "catalog-search-year-ids",
        "catalog-item-main",
        "catalog-item-creators",
        "catalog-item-tags",
        "catalog-item-attachments",
        "catalog-item-project-ids",
        "catalog-item-metadata-sources",
    ] {
        let file = format!("{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
}
