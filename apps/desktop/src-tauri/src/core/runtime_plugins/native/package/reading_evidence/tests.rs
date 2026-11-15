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
        package_tests::literature_fixture(b"synthetic reading_evidence host, never executed");
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
fn reading_evidence_methods_require_their_own_capability_without_new_core_requirements() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_reading_evidence(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": CAPABILITY_ID, "version": 1}));
        let key = select(&root, &catalog, &manifest);
        let package = accept_reading_evidence(&catalog, &key).unwrap();
        assert_eq!(package.package_key, key);
        // Metadata alone does not authorize missing fixed SQL artifacts.
        assert!(
            crate::runtime_plugins::reading_evidence::ReadingEvidenceResources::load(
                &catalog, &package
            )
            .is_err()
        );
    }
}

#[test]
fn reading_evidence_capability_does_not_accept_other_versions_duplicates_or_arbitrary_sql() {
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
                        json!("literature.reading_evidences.execute-sql");
                }
                2 => capabilities.last_mut().unwrap()["sql"] = json!(true),
                _ => capabilities.push(json!({"id": CAPABILITY_ID, "version": 1})),
            }
            let key = select(&root, &catalog, &candidate);
            assert_eq!(
                accept_reading_evidence(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn hbs_requires_both_finite_capabilities_without_changing_two_read_admission() {
    let (root, catalog, mut manifest) = fixture(false);
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":CAPABILITY_ID,"version":1}));
    let key = select(&root, &catalog, &manifest);
    assert!(accept_reading_evidence(&catalog, &key).is_ok());
    assert_eq!(
        accept_hbs_validation(&catalog, &key).err().unwrap().code,
        "plugin_host_capability_unavailable"
    );
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.structured-validation","version":1}));
    let key = select(&root, &catalog, &manifest);
    assert!(accept_hbs_validation(&catalog, &key).is_ok());
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .retain(|value| value["id"] != CAPABILITY_ID);
    let key = select(&root, &catalog, &manifest);
    assert_eq!(
        accept_hbs_validation(&catalog, &key).err().unwrap().code,
        "plugin_host_capability_unavailable"
    );
}

#[test]
fn unreviewed_selection_has_its_own_admission_without_unused_reading_resources() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_unreviewed_selection(&catalog, &key)
                .err()
                .unwrap()
                .code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":research_workbench_native_sdk::review_selection::CAPABILITY_ID,"version":1
        }));
        let key = select(&root, &catalog, &manifest);
        assert!(accept_unreviewed_selection(&catalog, &key).is_ok());
        assert_eq!(
            accept_hbs_validation(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
    }
}

#[test]
fn unreviewed_selection_rejects_unknown_versions_duplicates_and_sql_permissions() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":research_workbench_native_sdk::review_selection::CAPABILITY_ID,"version":1
        }));
        for variant in 0..4 {
            let mut candidate = manifest.clone();
            let values = candidate["provides"].as_array_mut().unwrap();
            match variant {
                0 => values.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    values.last_mut().unwrap()["id"] =
                        json!("literature.unreviewed-selection.execute-sql")
                }
                2 => values.last_mut().unwrap()["sql"] = json!(true),
                _ => values.push(values.last().unwrap().clone()),
            }
            let key = select(&root, &catalog, &candidate);
            assert_eq!(
                accept_unreviewed_selection(&catalog, &key)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn editable_template_is_independent_of_reading_sql_and_rejects_incompatible_grants() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_editable_template(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":research_workbench_native_sdk::editable_template::CAPABILITY_ID,"version":1
        }));
        let key = select(&root, &catalog, &manifest);
        assert!(accept_editable_template(&catalog, &key).is_ok());
        assert!(accept_hbs_validation(&catalog, &key).is_err());
        for variant in 0..3 {
            let mut changed = manifest.clone();
            let grants = changed["provides"].as_array_mut().unwrap();
            match variant {
                0 => grants.last_mut().unwrap()["version"] = json!(2),
                1 => grants.last_mut().unwrap()["sql"] = json!(true),
                _ => grants.push(grants.last().unwrap().clone()),
            }
            let key = select(&root, &catalog, &changed);
            assert_eq!(
                accept_editable_template(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn semantic_index_requires_its_exact_finite_capability_and_rejects_broader_grants() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert_eq!(
            accept_semantic_index(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":research_workbench_native_sdk::semantic_index::rules::CAPABILITY_ID,"version":1
        }));
        let key = select(&root, &catalog, &manifest);
        assert!(accept_semantic_index(&catalog, &key).is_ok());
        for variant in 0..4 {
            let mut changed = manifest.clone();
            let grants = changed["provides"].as_array_mut().unwrap();
            match variant {
                0 => grants.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    grants.last_mut().unwrap()["id"] =
                        json!("literature.semantic-index.execute-sql")
                }
                2 => grants.last_mut().unwrap()["sql"] = json!(true),
                _ => grants.push(grants.last().unwrap().clone()),
            }
            let key = select(&root, &catalog, &changed);
            assert_eq!(
                accept_semantic_index(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}
