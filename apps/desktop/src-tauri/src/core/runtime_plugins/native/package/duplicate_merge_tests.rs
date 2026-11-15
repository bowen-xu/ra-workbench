use super::*;
use research_workbench_native_sdk::{
    catalog, catalog_save, duplicate_merge_prepare, duplicate_review, duplicate_undo,
    literature_trace_protocol, nal_annotation_prepare, nal_bootstrap, nal_mutation, nal_query,
};
use serde_json::{Value, json};
use std::sync::Arc;

const HOST: &[u8] = b"synthetic merge host; never executed";

fn fixture(
    combined: bool,
    methods: &[&str],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(HOST);
    let directory = root.path().join("writing");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if combined {
        manifest = combined_tests::combined_manifest(&manifest);
        fs::write(directory.join("entry.js"), b"throw Error('never execute');").unwrap();
        fs::write(directory.join("style.css"), b"/* synthetic */").unwrap();
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
fn native_and_combined_merge_require_only_their_explicit_method_authority() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[duplicate_merge_prepare::CAPABILITY_ID]);
        let package = accept_duplicate_merge(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        unavailable(&accept_catalog(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_catalog_save(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_queries(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_bootstrap(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_annotation(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_mutation(&catalog, &key).err().unwrap(), &key);
        unavailable(
            &accept_duplicate_review(&catalog, &key).err().unwrap(),
            &key,
        );
        unavailable(&accept_duplicate_undo(&catalog, &key).err().unwrap(), &key);
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(fs::read(staged.executable).unwrap(), HOST);
    }
}

#[test]
fn existing_methods_do_not_implicitly_authorize_duplicate_merge() {
    let old_methods: [&[&str]; 6] = [
        &[],
        &[duplicate_review::CAPABILITY_ID],
        &[duplicate_undo::CAPABILITY_ID],
        &[nal_bootstrap::CAPABILITY_ID],
        &[nal_mutation::CAPABILITY_ID],
        &[
            catalog::CAPABILITY_ID,
            catalog_save::CAPABILITY_ID,
            nal_query::CAPABILITY_ID,
            nal_bootstrap::CAPABILITY_ID,
            nal_annotation_prepare::CAPABILITY_ID,
            nal_mutation::CAPABILITY_ID,
            duplicate_review::CAPABILITY_ID,
            duplicate_undo::CAPABILITY_ID,
        ],
    ];
    for combined in [false, true] {
        for methods in old_methods {
            let (_root, catalog, key, _) = fixture(combined, methods);
            accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            unavailable(&accept_duplicate_merge(&catalog, &key).err().unwrap(), &key);
        }
    }
}

#[test]
fn merge_can_coexist_with_each_independently_declared_literature_method() {
    for combined in [false, true] {
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
                duplicate_undo::CAPABILITY_ID,
                duplicate_merge_prepare::CAPABILITY_ID,
            ],
        );
        let package = accept_duplicate_merge(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 12 } else { 10 }
        );
        accept_catalog(&catalog, &key).unwrap();
        accept_catalog_save(&catalog, &key).unwrap();
        accept_nal_queries(&catalog, &key).unwrap();
        accept_nal_bootstrap(&catalog, &key).unwrap();
        accept_nal_annotation(&catalog, &key).unwrap();
        accept_nal_mutation(&catalog, &key).unwrap();
        accept_duplicate_review(&catalog, &key).unwrap();
        accept_duplicate_undo(&catalog, &key).unwrap();
        stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    }
}

#[test]
fn malformed_merge_and_sql_declarations_fail_both_accept_and_stage() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 7] = [
        ("duplicate merge", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":duplicate_merge_prepare::CAPABILITY_ID,"version":1}));
        }),
        ("merge version two", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
        }),
        ("unknown method", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.duplicate-merge.unknown");
        }),
        ("arbitrary SQL method", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.duplicate-merge.execute-sql");
        }),
        ("capability SQL field", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["sql"] = json!(true);
        }),
        ("manifest SQL declaration", |v| {
            v["sqlResources"] = json!(["arbitrary.sql"]);
        }),
        ("missing trace", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] != literature_trace_protocol::CAPABILITY_ID);
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) =
            fixture(combined, &[duplicate_merge_prepare::CAPABILITY_ID]);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_duplicate_merge(&catalog, &key).err().unwrap();
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
fn merge_does_not_replace_frontend_requirements_or_widen_writing_v1() {
    let (root, catalog, _, original) = fixture(true, &[duplicate_merge_prepare::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_duplicate_merge(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
        assert_eq!(
            stage_for(&catalog, &key, NativeProfile::NoteTraceDecision)
                .err()
                .unwrap()
                .code,
            "plugin_host_profile_unsupported"
        );
    }

    let (root, catalog, key) = tests::fixture(b"synthetic writing host; never executed");
    accept(&catalog, &key, NativeProfile::WritingPreview).unwrap();
    let original: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    type WritingRejectionCase = (fn(&mut Value), &'static str);
    let cases: [WritingRejectionCase; 2] = [
        (
            |value| {
                value["provides"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":duplicate_merge_prepare::CAPABILITY_ID,"version":1}));
            },
            "plugin_host_profile_unsupported",
        ),
        (
            |value| value["native"]["protocolVersion"] = json!(2),
            "plugin_host_protocol_incompatible",
        ),
    ];
    for (mutate, expected) in cases {
        let mut manifest = original.clone();
        mutate(&mut manifest);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept(&catalog, &key, NativeProfile::WritingPreview)
                .err()
                .unwrap()
                .code,
            expected
        );
        assert_eq!(
            stage_for(&catalog, &key, NativeProfile::WritingPreview)
                .err()
                .unwrap()
                .code,
            expected
        );
        assert_eq!(
            accept_duplicate_merge(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}
