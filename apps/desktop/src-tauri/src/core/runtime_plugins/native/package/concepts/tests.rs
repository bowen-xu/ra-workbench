use super::*;
use crate::runtime_plugins::native::package::{combined_tests, tests as package_tests};
use serde_json::{Value, json};

fn fixture(
    combined: bool,
) -> (
    tempfile::TempDir,
    std::sync::Arc<RuntimePluginCatalog>,
    Value,
) {
    let (root, catalog, _) =
        package_tests::literature_fixture(b"synthetic concept host, never executed");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    if combined {
        manifest = combined_tests::combined_manifest(&manifest);
        fs::write(root.path().join("writing/entry.js"), "export {};").unwrap();
        fs::write(root.path().join("writing/style.css"), "/* synthetic */").unwrap();
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

#[test]
fn concept_methods_require_their_own_capability_without_new_core_requirements() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_concepts(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": CAPABILITY_ID, "version": 1}));
        let key = select(&root, &catalog, &manifest);
        let package = accept_concepts(&catalog, &key).unwrap();
        assert_eq!(package.package_key, key);
        // Metadata alone does not authorize missing fixed SQL artifacts.
        assert!(
            crate::runtime_plugins::literature_concepts::ConceptResources::load(&catalog, &package)
                .is_err()
        );
    }
}

#[test]
fn concept_capability_does_not_accept_other_versions_duplicates_or_arbitrary_sql() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": CAPABILITY_ID, "version": 1}));
        for variant in 0..4 {
            let mut candidate = manifest.clone();
            let capabilities = candidate["provides"].as_array_mut().unwrap();
            match variant {
                0 => capabilities.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    capabilities.last_mut().unwrap()["id"] =
                        json!("literature.concepts.execute-sql");
                }
                2 => capabilities.last_mut().unwrap()["sql"] = json!(true),
                _ => capabilities.push(json!({"id": CAPABILITY_ID, "version": 1})),
            }
            let key = select(&root, &catalog, &candidate);
            assert_eq!(
                accept_concepts(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}
