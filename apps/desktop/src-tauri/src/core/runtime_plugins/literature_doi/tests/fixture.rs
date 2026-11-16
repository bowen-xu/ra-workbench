use super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog, literature_duplicates::tests::fixture::Fixture as CoreFixture,
};
use std::{fs, path::Path};

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub doi: DoiResources,
    pub jobs: ImportJobsResources,
    pub reads: CatalogResources,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let data = CoreFixture::new();
        let package_dir = tempfile::tempdir().unwrap();
        let root = package_dir.path().join("literature");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(root.join("index.js"), "export {};\n").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
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
            let filename = format!("{name}.v1.sql");
            fs::copy(source.join(&filename), root.join("data").join(filename)).unwrap();
        }
        let catalog = RuntimePluginCatalog::new(Some(package_dir.path().to_owned()));
        let package = catalog.discover().unwrap().packages.remove(0);
        let doi = DoiResources::load(&catalog, &package).unwrap();
        let jobs = ImportJobsResources::load(&catalog, &package).unwrap();
        let reads = CatalogResources::load(&catalog, &package).unwrap();
        data.db.lock().unwrap().execute_batch("create table synthetic_doi_observations(id integer primary key, phase text not null);").unwrap();
        Self {
            data,
            doi,
            jobs,
            reads,
            _package: package_dir,
        }
    }

    pub fn create(&self) -> (String, String) {
        storage::create_job(
            &mut self.data.db.lock().unwrap(),
            &self.jobs,
            "doi",
            "10.1000/normalized",
            " Raw DOI input ",
        )
        .unwrap()
    }

    pub fn preview(&self, job_id: &str) -> MetadataPreviewV1 {
        MetadataPreviewV1 {
            id: "synthetic-doi-candidate".into(), import_job_id: job_id.into(), item_id: Some("survivor".into()),
            source_kind: "doi.org:csl-json".into(), source_reference: "10.1000/normalized".into(),
            draft: serde_json::from_value(json!({"id":"survivor","itemType":"journalArticle","title":" Synthetic candidate ","doi":"10.1000/normalized","sourceKind":"doi.org:csl-json"})).unwrap(),
            diffs: Vec::new(), warnings: vec!["synthetic nonfatal warning".into()],
        }
    }

    pub fn sql(&self, sql: &str) {
        self.data.db.lock().unwrap().execute_batch(sql).unwrap();
    }

    pub fn scalar<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.data
            .db
            .lock()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }

    pub fn state(&self) -> Value {
        let db = self.data.db.lock().unwrap();
        json!({
            "jobs":rows(&db,"select json_array(id,state,source_kind,total_count,completed_count,cancellation_requested,created_at,updated_at) from import_jobs order by id"),
            "results":rows(&db,"select json_array(id,job_id,input_label,state,item_id,error_code,error_message,raw_input,created_at,updated_at) from import_results order by id"),
            "candidates":rows(&db,"select json_array(id,item_id,source_kind,source_reference,draft_json,raw_payload,import_result_id,state,created_at,decided_at) from metadata_candidates order by id"),
            "papers":rows(&db,"select json_array(id,item_type,title,abstract_text,issued_year,archived_at,trashed_at,created_at,updated_at) from bibliographic_items order by id"),
            "observations":rows(&db,"select json_array(id,phase) from synthetic_doi_observations order by id"),
            "note":serde_json::to_value(crate::notes::load_document_value(&db,crate::runtime_plugins::literature_duplicates::tests::fixture::PAGE).unwrap()).unwrap()
        })
    }

    pub fn assert_cold(self, expected: &Value) {
        let start = self.scalar::<String>("select strftime('%Y-%m-%dT%H:%M:%fZ','now')");
        let Self {
            data,
            doi,
            jobs,
            reads,
            _package,
        } = self;
        let reopened = Self {
            data: data.reopen(),
            doi,
            jobs,
            reads,
            _package,
        };
        let end = reopened.scalar::<String>("select strftime('%Y-%m-%dT%H:%M:%fZ','now')");
        assert!(start <= end);
        let actual = reopened.state();
        let mut recovered = expected.clone();
        assert_eq!(
            recovered["jobs"].as_array().unwrap().len(),
            actual["jobs"].as_array().unwrap().len()
        );
        for (before, after) in recovered["jobs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(actual["jobs"].as_array().unwrap())
        {
            if matches!(before[1].as_str(), Some("queued" | "running")) {
                assert_eq!(after[1], "interrupted");
                for i in [0, 2, 3, 4, 5, 6] {
                    assert_eq!(before[i], after[i]);
                }
                let time = after[7].as_str().unwrap();
                assert!(start.as_str() <= time && time <= end.as_str());
                before[1] = json!("interrupted");
                before[7] = json!(time);
            }
        }
        assert_eq!(actual, recovered);
        reopened.data.db.shutdown().unwrap();
    }
}

fn rows(db: &rusqlite::Connection, sql: &str) -> Vec<Value> {
    db.prepare(sql)
        .unwrap()
        .query_map([], |r| {
            let value: String = r.get(0)?;
            Ok(serde_json::from_str(&value).unwrap())
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
