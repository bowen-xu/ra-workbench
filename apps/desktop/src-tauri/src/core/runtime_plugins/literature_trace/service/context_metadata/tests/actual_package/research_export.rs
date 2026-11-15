//! Actual Core and independent package acceptance for the original complete export.
use super::*;
use crate::runtime_plugins::literature_export::{
    ResearchExportManager, ResearchExportSql, capture, writer,
};
use research_workbench_native_sdk::research_export::PreparedResearchExportV1;

async fn fixture() -> Fixture {
    let mut f = Fixture::new();
    super::super::export_fixture::seed_traceable_claim(&f.store.connection().unwrap());
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    f
}
async fn export(
    f: &Fixture,
    manager: &ResearchExportManager,
    target: &std::path::Path,
) -> NativeResult<crate::runtime_plugins::literature_export::ResearchExportReport> {
    let binding = f.store.native_note_trace().unwrap();
    f.service
        .research_export(
            f.store.clone(),
            binding.package_key.clone(),
            manager.clone(),
            target.to_str().unwrap().into(),
        )
        .await
}
fn files(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_research_export_preserves_all_datasets_bytes_provenance_and_original_publication()
 {
    let f = fixture().await;
    let target = tempfile::tempdir().unwrap();
    let manager = ResearchExportManager::default();
    let typed_snapshot = json!({
        "id": "claim-1",
        "statement": "A traceable claim",
        "positive": u64::MAX,
        "negative": i64::MIN,
        "negativeZero": -0.0,
        "fraction": 0.125,
        "small": 1e-12,
        "boolean": true,
        "optional": null,
        "array": ["Ω", "\\", "\n"],
    });
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into research_object_revisions(id,object_kind,object_id,revision,snapshot_json,editor_kind,change_reason,created_at) values('revision-typed','claim','claim-1',2,?1,'human','typed fixture','now')",
            [typed_snapshot.to_string()],
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("update claims set revision=2 where id='claim-1'", [])
        .unwrap();
    let report = export(&f, &manager, target.path()).await.unwrap();
    assert_eq!(report.file_count, 2);
    assert!(!manager.cancel());
    assert_eq!(files(target.path()).len(), 1);
    let folder = target.path().join(&report.folder_name);
    assert_eq!(files(&folder).len(), 2);
    let manifest = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
    let markdown = std::fs::read_to_string(folder.join("research-knowledge.md")).unwrap();
    let value: Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["scope"], "all_research_knowledge");
    assert_eq!(value["containsAiCandidateHistory"], true);
    assert_eq!(value["applicationVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["datasets"].as_array().unwrap().len(), 67);
    assert_eq!(value["objectCount"], report.object_count);
    assert_eq!(value["sourceCount"], report.source_count);
    let binding = f.store.native_note_trace().unwrap();
    let package = crate::runtime_plugins::native::package::accept_research_export(
        &f.service.catalog,
        &binding.package_key,
    )
    .unwrap();
    let sql = ResearchExportSql::load(&f.service.catalog, &package).unwrap();
    let snapshot = capture::capture(&f.store.connection().unwrap(), &sql, &manager).unwrap();
    assert_eq!(
        value["datasets"],
        serde_json::to_value(snapshot.datasets).unwrap()
    );
    assert!(manifest.contains("A traceable claim"));
    assert!(manifest.contains("\"negativeZero\": -0.0"));
    assert!(markdown.contains("Reading note text"));
    for forbidden in [
        "sourcePath",
        "private-paper.pdf",
        "apiKey",
        "aiProviderSettings",
    ] {
        assert!(!manifest.contains(forbidden));
    }
    let prepared = PreparedResearchExportV1 {
        exported_at: value["exportedAt"].as_str().unwrap().into(),
        manifest_json: manifest.clone(),
        markdown: markdown.clone(),
        object_count: report.object_count,
        source_count: report.source_count,
    };
    assert!(matches!(
        writer::write_export(target.path().to_str().unwrap(), &prepared, &manager),
        Err(crate::runtime_plugins::literature_export::ResearchExportError::DestinationExists)
    ));
    assert_eq!(files(target.path()).len(), 1);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(
        std::fs::read_to_string(folder.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("research-knowledge.md")).unwrap(),
        markdown
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select statement from claims where id='claim-1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "A traceable claim"
    );
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_research_export_invalid_target_and_capture_failure_publish_nothing_and_release_owner()
 {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let f = fixture().await;
    let target = tempfile::tempdir().unwrap();
    let manager = ResearchExportManager::default();
    assert_eq!(
        export(&f, &manager, &target.path().join("missing"))
            .await
            .unwrap_err()
            .code,
        "invalid_target"
    );
    assert!(!manager.cancel());
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "claims",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let failure = export(&f, &manager, target.path()).await.unwrap_err();
    assert_eq!(failure.code, "export_failed");
    assert!(failure.message.contains("cause="));
    assert!(files(target.path()).is_empty());
    assert!(!manager.cancel());
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(
        export(&f, &manager, target.path())
            .await
            .unwrap()
            .file_count,
        2
    );
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_research_export_original_record_and_publication_budgets_leave_no_files() {
    let f = fixture().await;
    let manager = ResearchExportManager::default();
    let target = tempfile::tempdir().unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "with recursive source(value) as (
            select 1 union all select value+1 from source where value<100001
        )
        insert into tags(id,name)
        select printf('export-limit-%d',value),printf('Export limit %d',value) from source;",
        )
        .unwrap();
    assert_eq!(
        export(&f, &manager, target.path()).await.unwrap_err().code,
        "too_large"
    );
    assert!(files(target.path()).is_empty());
    assert!(!manager.cancel());
    let prepared = PreparedResearchExportV1 {
        exported_at: "2026-11-13T00:00:00.000Z".into(),
        manifest_json: " "
            .repeat(research_workbench_native_sdk::research_export::MAX_EXPORT_BYTES + 1),
        markdown: String::new(),
        object_count: 0,
        source_count: 0,
    };
    assert!(matches!(
        writer::write_export(target.path().to_str().unwrap(), &prepared, &manager),
        Err(crate::runtime_plugins::literature_export::ResearchExportError::TooLarge)
    ));
    assert!(files(target.path()).is_empty());
    evidence_matrix::close(f).await;
}
