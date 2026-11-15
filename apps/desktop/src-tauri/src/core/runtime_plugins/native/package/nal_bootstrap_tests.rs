use super::*;
use research_workbench_native_sdk::{catalog, catalog_save, nal_bootstrap, nal_query};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(
    combined: bool,
    methods: &[&str],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"synthetic host bytes; never executed");
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
        fs::write(package.join("style.css"), b"/* fixture style */").unwrap();
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

fn unavailable(error: &NativePreviewError, key: &str) {
    assert_eq!(error.code, "plugin_host_capability_unavailable");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(key));
}

#[test]
fn explicit_bootstrap_is_independent_of_catalog_and_query_permissions() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[nal_bootstrap::CAPABILITY_ID]);
        let accepted = accept_nal_bootstrap(&catalog, &key).unwrap();
        assert_eq!(
            accepted.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        unavailable(&accept_catalog(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_catalog_save(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_queries(&catalog, &key).err().unwrap(), &key);
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
        assert_eq!(
            fs::read(staged.executable).unwrap(),
            b"synthetic host bytes; never executed"
        );
    }
}

#[test]
fn all_preexisting_native_and_combined_sets_keep_their_original_authority_without_bootstrap() {
    let old_sets = [
        vec![],
        vec![catalog::CAPABILITY_ID],
        vec![catalog::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        vec![nal_query::CAPABILITY_ID],
        vec![nal_query::CAPABILITY_ID, catalog::CAPABILITY_ID],
        vec![
            nal_query::CAPABILITY_ID,
            catalog::CAPABILITY_ID,
            catalog_save::CAPABILITY_ID,
        ],
    ];
    for combined in [false, true] {
        for methods in &old_sets {
            let (_root, catalog, key, _) = fixture(combined, methods);
            accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            unavailable(&accept_nal_bootstrap(&catalog, &key).err().unwrap(), &key);
            assert_eq!(
                accept_catalog(&catalog, &key).is_ok(),
                methods.contains(&catalog::CAPABILITY_ID)
            );
            assert_eq!(
                accept_catalog_save(&catalog, &key).is_ok(),
                methods.contains(&catalog_save::CAPABILITY_ID)
            );
            assert_eq!(
                accept_nal_queries(&catalog, &key).is_ok(),
                methods.contains(&nal_query::CAPABILITY_ID)
            );
        }
    }
}

#[test]
fn bootstrap_can_coexist_with_the_other_explicit_methods_but_does_not_replace_catalog_reads() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(
            combined,
            &[
                nal_bootstrap::CAPABILITY_ID,
                nal_query::CAPABILITY_ID,
                catalog::CAPABILITY_ID,
                catalog_save::CAPABILITY_ID,
            ],
        );
        accept_nal_bootstrap(&catalog, &key).unwrap();
        accept_nal_queries(&catalog, &key).unwrap();
        accept_catalog(&catalog, &key).unwrap();
        accept_catalog_save(&catalog, &key).unwrap();
        let (_root, catalog, key, _) = fixture(
            combined,
            &[nal_bootstrap::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        );
        assert_eq!(
            accept_nal_bootstrap(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn duplicate_unknown_version_extra_authority_and_missing_trace_are_rejected_at_both_entries() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 5] = [
        ("duplicate", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":nal_bootstrap::CAPABILITY_ID,"version":1}))
        }),
        ("version2", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2)
        }),
        ("unknown", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.nal-arbitrary-step")
        }),
        ("extra authority", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["sql"] = json!(true)
        }),
        ("missing trace", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] != "literature.note-trace-decision")
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, &[nal_bootstrap::CAPABILITY_ID]);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_nal_bootstrap(&catalog, &key).err().unwrap();
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
fn bootstrap_does_not_remove_the_two_required_frontend_provides_of_a_combined_package() {
    let (root, catalog, _, original) = fixture(true, &[nal_bootstrap::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_nal_bootstrap(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported",
            "{missing}"
        );
    }
}

#[test]
fn bootstrap_cannot_widen_the_original_writing_profile() {
    let (root, catalog, _) = tests::fixture(b"synthetic writing host bytes");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":nal_bootstrap::CAPABILITY_ID,"version":1}));
    let key = write_manifest(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        accept_nal_bootstrap(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}
