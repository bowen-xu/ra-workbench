use super::*;
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(combined: bool) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"synthetic metadata host; never executed");
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

fn declarations(manifest: &mut Value) {
    for id in [
        "literature.catalog-read",
        "literature.catalog-save",
        "literature.metadata",
    ] {
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
}

#[test]
fn metadata_requires_explicit_preparation_and_original_save_projection_without_granting_sql() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_metadata(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        // A different declared method never implies this metadata authority.
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.lifecycle","version":1}));
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_metadata(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        declarations(&mut manifest);
        let key = select(&root, &catalog, &manifest);
        let accepted = accept_metadata(&catalog, &key).unwrap();
        assert!(
            crate::runtime_plugins::literature_metadata::MetadataResources::load(
                &catalog, &accepted
            )
            .is_err()
        );
        for missing in ["literature.metadata", "literature.catalog-save"] {
            let mut incomplete = manifest.clone();
            incomplete["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] != missing);
            let key = select(&root, &catalog, &incomplete);
            assert_eq!(
                accept_metadata(&catalog, &key).err().unwrap().code,
                "plugin_host_capability_unavailable"
            );
        }
    }
}

#[test]
fn metadata_rejects_undeclared_wire_fields_duplicates_and_versions_for_both_package_forms() {
    for combined in [false, true] {
        let (root, catalog, mut original) = fixture(combined);
        declarations(&mut original);
        for variant in 0..4 {
            let mut manifest = original.clone();
            let provides = manifest["provides"].as_array_mut().unwrap();
            match variant {
                0 => provides.last_mut().unwrap()["version"] = json!(2),
                1 => provides.last_mut().unwrap()["id"] = json!("literature.metadata.execute-sql"),
                2 => provides.last_mut().unwrap()["sql"] = json!(true),
                _ => provides.push(json!({"id":"literature.metadata","version":1})),
            }
            let key = select(&root, &catalog, &manifest);
            assert_eq!(
                accept_metadata(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}
