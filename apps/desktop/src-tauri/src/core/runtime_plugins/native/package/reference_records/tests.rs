use super::*;
use serde_json::{Value, json};

fn fixture(
    combined: bool,
) -> (
    tempfile::TempDir,
    std::sync::Arc<RuntimePluginCatalog>,
    Value,
) {
    let (root, catalog, _) =
        super::super::tests::literature_fixture(b"synthetic reference host; never executed");
    let directory = root.path().join("writing");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if combined {
        manifest = super::super::combined_tests::combined_manifest(&manifest);
        fs::write(
            directory.join("entry.js"),
            b"throw Error('never execute');\n",
        )
        .unwrap();
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

#[test]
fn original_reference_methods_need_their_own_capability_and_fixed_resources() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_reference_records(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.lifecycle","version":1}));
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_reference_records(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        let key = select(&root, &catalog, &manifest);
        let package = accept_reference_records(&catalog, &key).unwrap();
        assert_eq!(package.package_key, key);
        // Declaring the method is not authority for missing SQL resources.
        assert!(
            crate::runtime_plugins::literature_reference_records::ReferenceResources::load(
                &catalog, &package
            )
            .is_err()
        );
    }
}

#[test]
fn reference_capability_rejects_unknown_versions_duplicates_and_sql_fields() {
    for combined in [false, true] {
        let (root, catalog, mut original) = fixture(combined);
        original["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        for variant in 0..4 {
            let mut manifest = original.clone();
            let provides = manifest["provides"].as_array_mut().unwrap();
            match variant {
                0 => provides.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    provides.last_mut().unwrap()["id"] =
                        json!("literature.reference-records.execute-sql")
                }
                2 => provides.last_mut().unwrap()["sql"] = json!(true),
                _ => provides.push(json!({"id":CAPABILITY_ID,"version":1})),
            }
            let key = select(&root, &catalog, &manifest);
            assert_eq!(
                accept_reference_records(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}
