use super::*;
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(combined: bool) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, Value) {
    let (root, catalog, _) =
        tests::literature_fixture(b"synthetic import-jobs host; never executed");
    let directory = root.path().join("writing");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if combined {
        manifest = combined_tests::combined_manifest(&manifest);
        fs::write(directory.join("entry.js"), b"throw Error('never execute');").unwrap();
        fs::write(directory.join("style.css"), b"/* synthetic */").unwrap();
    }
    (root, catalog, manifest)
}

fn select(root: &tempfile::TempDir, catalog: &RuntimePluginCatalog, manifest: &Value) -> String {
    fs::write(
        root.path().join("writing/manifest.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    catalog.discover().unwrap().packages.remove(0).package_key
}

fn declare(manifest: &mut Value, id: &str) {
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":id,"version":1}));
}

#[test]
fn import_job_reads_and_cancel_do_not_require_unrelated_catalog_save_authority() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert!(accept(&catalog, &key, NativeProfile::NoteTraceDecision).is_ok());
        assert_eq!(
            accept_import_jobs(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        declare(&mut manifest, "literature.import-jobs");
        let key = select(&root, &catalog, &manifest);
        let accepted = accept_import_jobs(&catalog, &key).unwrap();
        // Declaring a method does not grant or manufacture any of its fixed SQL.
        let missing = crate::runtime_plugins::literature_import_jobs::ImportJobsResources::load(
            &catalog, &accepted,
        )
        .err()
        .unwrap();
        assert_eq!(missing.code, "runtime_plugin_resource_unknown");
        assert_eq!(
            accept_bibtex(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        declare(&mut manifest, "literature.catalog-read");
        let key = select(&root, &catalog, &manifest);
        assert!(accept_import_jobs(&catalog, &key).is_ok());
        assert_eq!(
            accept_bibtex(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        declare(&mut manifest, "literature.catalog-save");
        let key = select(&root, &catalog, &manifest);
        assert!(accept_bibtex(&catalog, &key).is_ok());
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != "literature.import-jobs");
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_bibtex(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
    }
}

#[test]
fn import_job_capabilities_reject_unknown_permissions_versions_and_duplicates() {
    for combined in [false, true] {
        let (root, catalog, mut original) = fixture(combined);
        declare(&mut original, "literature.import-jobs");
        for change in 0..4 {
            let mut manifest = original.clone();
            let values = manifest["provides"].as_array_mut().unwrap();
            match change {
                0 => values.last_mut().unwrap()["version"] = json!(2),
                1 => values.last_mut().unwrap()["id"] = json!("literature.import-jobs.execute-sql"),
                2 => values.last_mut().unwrap()["sql"] = json!(true),
                _ => values.push(json!({"id":"literature.import-jobs","version":1})),
            }
            let key = select(&root, &catalog, &manifest);
            assert_eq!(
                accept_import_jobs(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn old_literature_method_sets_stay_compatible_and_writing_remains_closed() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        for id in [
            "literature.catalog-read",
            "literature.catalog-save",
            "literature.nal-query-collections",
            "literature.nal-bootstrap",
            "literature.nal-annotation",
            "literature.nal-mutation",
            "literature.duplicate-review",
            "literature.duplicate-undo",
            "literature.duplicate-merge",
            "literature.lifecycle",
            "literature.trash-cleanup",
            "literature.metadata",
        ] {
            declare(&mut manifest, id);
        }
        let key = select(&root, &catalog, &manifest);
        assert!(accept(&catalog, &key, NativeProfile::NoteTraceDecision).is_ok());
        assert_eq!(
            manifest["provides"].as_array().unwrap().len(),
            if combined { 15 } else { 13 }
        );
        assert_eq!(
            accept_import_jobs(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
    }
    let (root, catalog, _) = tests::fixture(b"synthetic writing host; never executed");
    let file = root.path().join("writing/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    declare(&mut manifest, "literature.import-jobs");
    let key = select(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
}
