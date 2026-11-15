use super::*;
use research_workbench_native_sdk::trash_cleanup::CAPABILITY_ID;
use serde_json::{Value, json};
use std::sync::Arc;

fn package(combined: bool) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, Value) {
    let (root, catalog, _) =
        tests::literature_fixture(b"synthetic trash cleanup host; never executed");
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

fn key(root: &tempfile::TempDir, catalog: &RuntimePluginCatalog, manifest: &Value) -> String {
    fs::write(
        root.path().join("writing/manifest.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    catalog.discover().unwrap().packages.remove(0).package_key
}

#[test]
fn trash_cleanup_requires_its_own_authority_and_does_not_grant_other_methods_or_sql() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = package(combined);
        for other in [
            None,
            Some("literature.duplicate-merge"),
            Some("literature.duplicate-review"),
        ] {
            let mut old = manifest.clone();
            if let Some(id) = other {
                old["provides"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":id,"version":1}));
            }
            let selected = key(&root, &catalog, &old);
            let error = accept_trash_cleanup(&catalog, &selected).err().unwrap();
            assert_eq!(error.code, "plugin_host_capability_unavailable");
            assert_eq!(error.package_key.as_deref(), Some(selected.as_str()));
        }
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        let selected = key(&root, &catalog, &manifest);
        let accepted = accept_trash_cleanup(&catalog, &selected).unwrap();
        assert_eq!(
            accept_duplicate_merge(&catalog, &selected)
                .err()
                .unwrap()
                .code,
            "plugin_host_capability_unavailable"
        );
        assert_eq!(
            accept_catalog(&catalog, &selected).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        assert!(
            crate::runtime_plugins::literature_trash_cleanup::TrashCleanupResources::load(
                &catalog, &accepted
            )
            .is_err(),
            "method declaration cannot manufacture the required fixed SQL resources"
        );
    }
}

#[test]
fn trash_cleanup_retains_the_closed_versions_fields_and_writing_profile() {
    for combined in [false, true] {
        let (root, catalog, mut original) = package(combined);
        original["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        for mutation in 0..5 {
            let mut manifest = original.clone();
            match mutation {
                0 => {
                    manifest["provides"]
                        .as_array_mut()
                        .unwrap()
                        .last_mut()
                        .unwrap()["version"] = json!(2)
                }
                1 => {
                    manifest["provides"]
                        .as_array_mut()
                        .unwrap()
                        .last_mut()
                        .unwrap()["id"] = json!("literature.trash-cleanup.execute-sql")
                }
                2 => {
                    manifest["provides"]
                        .as_array_mut()
                        .unwrap()
                        .last_mut()
                        .unwrap()["sql"] = json!(true)
                }
                3 => {
                    manifest["provides"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|v| v["id"] != "literature.note-trace-decision");
                }
                _ => {
                    manifest["provides"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"id":CAPABILITY_ID,"version":1}));
                }
            }
            let selected = key(&root, &catalog, &manifest);
            assert_eq!(
                accept_trash_cleanup(&catalog, &selected)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
    let (root, catalog, _) = tests::fixture(b"synthetic writing host; never executed");
    let file = root.path().join("writing/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":CAPABILITY_ID,"version":1}));
    let selected = key(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &selected, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
}
