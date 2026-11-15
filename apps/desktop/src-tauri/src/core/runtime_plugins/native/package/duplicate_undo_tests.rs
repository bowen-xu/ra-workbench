use super::*;
use crate::runtime_plugins::data::AcceptedDataResource;
use research_workbench_native_sdk::{
    catalog, catalog_save,
    data::{DataAccess, DataField, DataResourceGrant, DataType},
    duplicate_review, duplicate_undo, nal_annotation_prepare, nal_bootstrap, nal_mutation,
    nal_query,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(
    combined: bool,
    methods: &[&str],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"synthetic undo host; never executed");
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

const HEADER_PATH: &str = "data/duplicate-undo-header.v1.sql";

fn copy_header(root: &tempfile::TempDir) -> Vec<u8> {
    let bytes = fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native")
            .join(HEADER_PATH),
    )
    .unwrap();
    fs::write(root.path().join("writing").join(HEADER_PATH), &bytes).unwrap();
    bytes
}

fn header_grant() -> DataResourceGrant {
    let text = DataField {
        data_type: DataType::Text,
        nullable: false,
    };
    DataResourceGrant {
        id: "literature.duplicate-undo.header".into(),
        version: 1,
        sha256: "c3f06f758ae8f61008cf87967b18a874c6c7ef8f3996d21613733901921257ee".into(),
        access: DataAccess::Read,
        parameters: vec![text],
        columns: vec![text; 3],
    }
}

#[test]
fn native_and_combined_undo_require_explicit_method_authority() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[duplicate_undo::CAPABILITY_ID]);
        let package = accept_duplicate_undo(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        unavailable(
            &accept_duplicate_review(&catalog, &key).err().unwrap(),
            &key,
        );
        unavailable(&accept_nal_bootstrap(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_mutation(&catalog, &key).err().unwrap(), &key);
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(
            fs::read(staged.executable).unwrap(),
            b"synthetic undo host; never executed"
        );

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
            ],
        );
        let package = accept_duplicate_undo(&catalog, &key).unwrap();
        assert_eq!(
            package.manifest["provides"].as_array().unwrap().len(),
            if combined { 11 } else { 9 }
        );
        accept_duplicate_review(&catalog, &key).unwrap();
        accept_nal_bootstrap(&catalog, &key).unwrap();
        accept_nal_mutation(&catalog, &key).unwrap();
    }
}

#[test]
fn existing_review_bootstrap_mutation_and_catalog_packages_do_not_authorize_undo() {
    let old_methods: [&[&str]; 5] = [
        &[],
        &[duplicate_review::CAPABILITY_ID],
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
        ],
    ];
    for combined in [false, true] {
        for methods in old_methods {
            let (_root, catalog, key, _) = fixture(combined, methods);
            accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            unavailable(&accept_duplicate_undo(&catalog, &key).err().unwrap(), &key);
        }
    }
}

#[test]
fn malformed_undo_capabilities_and_arbitrary_sql_metadata_are_rejected() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 6] = [
        ("duplicate", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":duplicate_undo::CAPABILITY_ID,"version":1}));
        }),
        ("wrong version", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
        }),
        ("arbitrary SQL capability", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.duplicate-undo.execute-sql");
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
                .retain(|entry| entry["id"] != "literature.note-trace-decision");
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, &[duplicate_undo::CAPABILITY_ID]);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_duplicate_undo(&catalog, &key).err().unwrap();
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
fn undo_does_not_replace_frontend_requirements_or_widen_writing_v1() {
    let (root, catalog, _, original) = fixture(true, &[duplicate_undo::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_duplicate_undo(&catalog, &key).err().unwrap().code,
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
        .push(json!({"id":duplicate_undo::CAPABILITY_ID,"version":1}));
    let key = write_manifest(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        accept_duplicate_undo(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}

#[test]
fn accepting_a_fixed_header_resource_does_not_grant_the_undo_method() {
    let (root, catalog, _, manifest) = fixture(false, &[]);
    copy_header(&root);
    let key = write_manifest(&root, &catalog, &manifest);
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let resource = AcceptedDataResource::load(
        &catalog,
        &package,
        "literature_library",
        HEADER_PATH,
        header_grant(),
    )
    .unwrap();
    assert_eq!(resource.owner_id(), "literature_library");
    unavailable(&accept_duplicate_undo(&catalog, &key).err().unwrap(), &key);
}

#[test]
fn changed_header_bytes_cannot_reuse_the_original_package_snapshot_or_fixed_grant() {
    let (root, catalog, _, manifest) = fixture(false, &[duplicate_undo::CAPABILITY_ID]);
    let original = copy_header(&root);
    let key = write_manifest(&root, &catalog, &manifest);
    let package = accept_duplicate_undo(&catalog, &key).unwrap();
    let load = |package: &RuntimePluginPackage| {
        AcceptedDataResource::load(
            &catalog,
            package,
            "literature_library",
            HEADER_PATH,
            header_grant(),
        )
    };
    let accepted = load(&package).unwrap();
    let mut changed = original.clone();
    changed.push(b'\n');
    fs::write(root.path().join("writing").join(HEADER_PATH), changed).unwrap();
    assert_eq!(
        load(&package).err().unwrap().code,
        "runtime_plugin_resource_changed"
    );
    assert_eq!(accepted.sql().as_bytes(), original);
    let changed_key = write_manifest(&root, &catalog, &manifest);
    let changed_package = accept_duplicate_undo(&catalog, &changed_key).unwrap();
    assert_eq!(
        load(&changed_package).err().unwrap().code,
        "runtime_plugin_data_digest_mismatch"
    );
}
