use super::*;
use research_workbench_native_sdk::{
    catalog, catalog_save, duplicate_review, nal_annotation_prepare, nal_bootstrap, nal_mutation,
    nal_query,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(
    combined: bool,
    methods: &[&str],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"synthetic package host; never executed");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    if combined {
        manifest = combined_tests::combined_manifest(&manifest);
        fs::write(
            root.path().join("writing/entry.js"),
            b"throw Error('never execute');",
        )
        .unwrap();
        fs::write(root.path().join("writing/style.css"), b"/* synthetic */").unwrap();
    }
    for id in methods {
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
    let key = write_manifest(&root, &catalog, &manifest);
    (root, catalog, key, manifest)
}

fn write_manifest(
    root: &tempfile::TempDir,
    catalog: &RuntimePluginCatalog,
    manifest: &Value,
) -> String {
    fs::write(
        root.path().join("writing/manifest.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    listing.packages[0].package_key.clone()
}

fn unavailable(error: &NativeHostError, key: &str) {
    assert_eq!(error.code, "plugin_host_capability_unavailable");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(key));
}

#[test]
fn native_and_combined_review_is_explicit_and_does_not_imply_other_methods() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[duplicate_review::CAPABILITY_ID]);
        let package = accept_duplicate_review(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        unavailable(&accept_catalog(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_catalog_save(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_mutation(&catalog, &key).err().unwrap(), &key);

        let (_root, catalog, key, _) = fixture(
            combined,
            &[
                catalog::CAPABILITY_ID,
                catalog_save::CAPABILITY_ID,
                nal_query::CAPABILITY_ID,
                nal_bootstrap::CAPABILITY_ID,
                nal_annotation_prepare::CAPABILITY_ID,
                nal_mutation::CAPABILITY_ID,
                duplicate_review::CAPABILITY_ID,
            ],
        );
        let package = accept_duplicate_review(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 10 } else { 8 }
        );
        accept_catalog_save(&catalog, &key).unwrap();
        accept_nal_mutation(&catalog, &key).unwrap();
    }
}

#[test]
fn absent_review_is_unavailable_without_changing_existing_trace_and_catalog_routes() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[]);
        accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        unavailable(
            &accept_duplicate_review(&catalog, &key).err().unwrap(),
            &key,
        );

        let (_root, catalog, key, _) = fixture(
            combined,
            &[catalog::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        );
        accept_catalog(&catalog, &key).unwrap();
        accept_catalog_save(&catalog, &key).unwrap();
        unavailable(
            &accept_duplicate_review(&catalog, &key).err().unwrap(),
            &key,
        );
    }
}

#[test]
fn review_versions_duplicates_unknown_methods_and_sql_declarations_are_rejected() {
    let mutations: [fn(&mut Value); 5] = [
        |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":duplicate_review::CAPABILITY_ID,"version":1}));
        },
        |v| v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2),
        |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.duplicate-review.execute-sql");
        },
        |v| v["provides"].as_array_mut().unwrap().last_mut().unwrap()["sql"] = json!(true),
        |v| v["sqlResources"] = json!(["arbitrary.sql"]),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, &[duplicate_review::CAPABILITY_ID]);
        for mutate in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_duplicate_review(&catalog, &key).err().unwrap();
            assert_eq!(error.code, "plugin_host_profile_unsupported");
            assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
            assert_eq!(
                stage_for(&catalog, &key, NativeProfile::NoteTraceDecision)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn review_does_not_replace_combined_frontend_requirements_or_widen_writing_v1() {
    let (root, catalog, _, original) = fixture(true, &[duplicate_review::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_duplicate_review(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
    let (root, catalog, key) = tests::fixture(b"synthetic writing host; never executed");
    accept(&catalog, &key, NativeProfile::WritingPreview).unwrap();
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":duplicate_review::CAPABILITY_ID,"version":1}));
    let key = write_manifest(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        accept_duplicate_review(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}
