use super::*;
use research_workbench_native_sdk::{catalog, catalog_save, nal_query};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(
    combined: bool,
    methods: &[&str],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"synthetic host; never executed");
    let package = root.path().join("writing");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    if combined {
        manifest = combined_tests::combined_manifest(&manifest);
        fs::write(
            package.join("entry.js"),
            b"throw new Error('never execute');",
        )
        .unwrap();
        fs::write(package.join("style.css"), b"/* synthetic stylesheet */").unwrap();
    }
    for id in methods {
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in ["nal-query-read.v1.sql", "nal-query-save.v1.sql"] {
        fs::copy(source.join(name), package.join("data").join(name)).unwrap();
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

fn assert_unavailable(error: &NativePreviewError, key: &str) {
    assert_eq!(error.code, "plugin_host_capability_unavailable");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(key));
}

#[test]
fn explicit_query_capability_is_independent_of_catalog_read_and_save_authority() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[nal_query::CAPABILITY_ID]);
        let package = accept_nal_queries(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        assert_unavailable(&accept_catalog(&catalog, &key).err().unwrap(), &key);
        assert_unavailable(&accept_catalog_save(&catalog, &key).err().unwrap(), &key);
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
        assert_eq!(
            fs::read(staged.executable).unwrap(),
            b"synthetic host; never executed"
        );
    }
}

#[test]
fn prior_native_one_two_three_and_combined_three_four_five_keep_original_permissions() {
    let original_sets: [&[&str]; 3] = [
        &[],
        &[catalog::CAPABILITY_ID],
        &[catalog::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
    ];
    for combined in [false, true] {
        for methods in original_sets {
            let (_root, catalog, key, _) = fixture(combined, methods);
            let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            assert_eq!(
                package.manifest["provides"].as_array().unwrap().len(),
                methods.len() + if combined { 3 } else { 1 }
            );
            assert_unavailable(&accept_nal_queries(&catalog, &key).err().unwrap(), &key);
            assert_eq!(accept_catalog(&catalog, &key).is_ok(), !methods.is_empty());
            assert_eq!(
                accept_catalog_save(&catalog, &key).is_ok(),
                methods.len() == 2
            );
        }
    }
}

#[test]
fn query_can_coexist_with_catalog_but_does_not_replace_saves_required_read() {
    for combined in [false, true] {
        for methods in [
            vec![nal_query::CAPABILITY_ID, catalog::CAPABILITY_ID],
            vec![
                catalog_save::CAPABILITY_ID,
                nal_query::CAPABILITY_ID,
                catalog::CAPABILITY_ID,
            ],
        ] {
            let (_root, catalog, key, _) = fixture(combined, &methods);
            accept_nal_queries(&catalog, &key).unwrap();
            accept_catalog(&catalog, &key).unwrap();
            assert_eq!(
                accept_catalog_save(&catalog, &key).is_ok(),
                methods.len() == 3
            );
        }
        let (_root, catalog, key, _) = fixture(
            combined,
            &[nal_query::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        );
        assert_eq!(
            accept_nal_queries(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn query_capability_rejects_duplicate_unknown_version_and_extra_authority_at_both_entries() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 5] = [
        ("duplicate query", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":nal_query::CAPABILITY_ID,"version":1}));
        }),
        ("version two", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
        }),
        ("unknown capability", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.nal-query-arbitrary-sql");
        }),
        ("extra method permission", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["methods"] =
                json!(["anything"]);
        }),
        ("missing trace", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] != json!("literature.note-trace-decision"));
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, &[nal_query::CAPABILITY_ID]);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_nal_queries(&catalog, &key).err().unwrap();
            assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
            assert_eq!(error.package_key.as_deref(), Some(key.as_str()), "{name}");
            assert_eq!(
                stage_for(&catalog, &key, NativeProfile::NoteTraceDecision)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported",
                "{name}"
            );
        }
    }
}

#[test]
fn combined_query_still_requires_both_original_frontend_capabilities() {
    let (root, catalog, _, original) = fixture(true, &[nal_query::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_nal_queries(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported",
            "{missing}"
        );
        assert_eq!(
            stage_for(&catalog, &key, NativeProfile::NoteTraceDecision)
                .err()
                .unwrap()
                .code,
            "plugin_host_profile_unsupported",
            "{missing}"
        );
    }
}

#[test]
fn query_declaration_cannot_expand_the_original_writing_profile() {
    let (root, catalog, _) = tests::fixture(b"synthetic writing host");
    let file = root.path().join("writing/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":nal_query::CAPABILITY_ID,"version":1}));
    let key = write_manifest(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        accept_nal_queries(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}
