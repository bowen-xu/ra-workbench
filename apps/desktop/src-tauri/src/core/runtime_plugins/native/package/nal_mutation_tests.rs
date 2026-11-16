use super::*;
use crate::runtime_plugins::data::AcceptedDataResource;
use research_workbench_native_sdk::{
    catalog, catalog_save,
    data::{DataAccess, DataField, DataResourceGrant, DataType},
    nal_annotation_prepare, nal_bootstrap, nal_mutation, nal_query,
};
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
        fs::write(package.join("style.css"), b"/* synthetic style */").unwrap();
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
fn explicit_mutation_capability_does_not_imply_other_method_authority() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, &[nal_mutation::CAPABILITY_ID]);
        let accepted = accept_nal_mutation(&catalog, &key).unwrap();
        assert_eq!(
            accepted.manifest["provides"].as_array().unwrap().len(),
            if combined { 4 } else { 2 }
        );
        unavailable(&accept_catalog(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_catalog_save(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_queries(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_bootstrap(&catalog, &key).err().unwrap(), &key);
        unavailable(&accept_nal_annotation(&catalog, &key).err().unwrap(), &key);
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(
            fs::read(staged.executable).unwrap(),
            b"synthetic host bytes; never executed"
        );
    }
}

#[test]
fn prior_native_and_combined_methods_do_not_implicitly_authorize_mutation() {
    let old_sets: [&[&str]; 9] = [
        &[],
        &[catalog::CAPABILITY_ID],
        &[catalog::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        &[nal_query::CAPABILITY_ID],
        &[nal_bootstrap::CAPABILITY_ID],
        &[nal_query::CAPABILITY_ID, nal_bootstrap::CAPABILITY_ID],
        &[nal_annotation_prepare::CAPABILITY_ID],
        &[
            nal_bootstrap::CAPABILITY_ID,
            nal_annotation_prepare::CAPABILITY_ID,
        ],
        &[
            catalog::CAPABILITY_ID,
            catalog_save::CAPABILITY_ID,
            nal_query::CAPABILITY_ID,
            nal_bootstrap::CAPABILITY_ID,
            nal_annotation_prepare::CAPABILITY_ID,
        ],
    ];
    for combined in [false, true] {
        for methods in old_sets {
            let (_root, catalog, key, _) = fixture(combined, methods);
            accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            unavailable(&accept_nal_mutation(&catalog, &key).err().unwrap(), &key);
        }
    }
}

#[test]
fn seven_native_or_nine_combined_capabilities_keep_catalog_saves_read_requirement() {
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
            ],
        );
        let accepted = accept_nal_mutation(&catalog, &key).unwrap();
        assert_eq!(
            accepted.manifest["provides"].as_array().unwrap().len(),
            if combined { 9 } else { 7 }
        );
        accept_catalog_save(&catalog, &key).unwrap();
        accept_nal_queries(&catalog, &key).unwrap();
        accept_nal_bootstrap(&catalog, &key).unwrap();
        accept_nal_annotation(&catalog, &key).unwrap();
        let (_root, catalog, key, _) = fixture(
            combined,
            &[nal_mutation::CAPABILITY_ID, catalog_save::CAPABILITY_ID],
        );
        assert_eq!(
            accept_nal_mutation(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn malformed_or_extra_sql_authority_is_rejected_by_acceptance_and_staging() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 7] = [
        ("duplicate", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":nal_mutation::CAPABILITY_ID,"version":1}));
        }),
        ("version2", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
        }),
        ("unknown method", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.nal-mutation-arbitrary-sql");
        }),
        ("method sql field", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["sql"] = json!(true);
        }),
        ("manifest sql declaration", |v| {
            v["sqlResources"] = json!(["arbitrary.sql"]);
        }),
        ("native grants", |v| {
            v["native"]["grants"] = json!(["anything"]);
        }),
        ("missing trace", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] != "literature.note-trace-decision");
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, &[nal_mutation::CAPABILITY_ID]);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_nal_mutation(&catalog, &key).err().unwrap();
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
fn a_fixed_sql_resource_grant_does_not_authorize_the_mutation_method() {
    let (root, catalog, _, manifest) = fixture(false, &[]);
    let relative = "data/nal-mutation-term-active-concept.v1.sql";
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature")
            .join("nal-mutation-term-active-concept.v1.sql"),
        root.path().join("writing").join(relative),
    )
    .unwrap();
    let key = write_manifest(&root, &catalog, &manifest);
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let grant = DataResourceGrant {
        id: "literature.nal-mutation.term-active-concept".into(),
        version: 1,
        sha256: "dc006d1a22f0d72ad9767ae89373219e3d32720598011ccece98b55fa885fa47".into(),
        access: DataAccess::Read,
        parameters: vec![DataField {
            data_type: DataType::Text,
            nullable: false,
        }],
        columns: vec![DataField {
            data_type: DataType::Integer,
            nullable: false,
        }],
    };
    let accepted =
        AcceptedDataResource::load(&catalog, &package, "literature_library", relative, grant)
            .unwrap();
    assert_eq!(accepted.owner_id(), "literature_library");
    unavailable(&accept_nal_mutation(&catalog, &key).err().unwrap(), &key);
}

#[test]
fn mutation_does_not_replace_frontend_requirements_or_widen_writing_v1() {
    let (root, catalog, _, original) = fixture(true, &[nal_mutation::CAPABILITY_ID]);
    for missing in ["literature.catalog", "literature.citations"] {
        let mut manifest = original.clone();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["id"] != missing);
        let key = write_manifest(&root, &catalog, &manifest);
        assert_eq!(
            accept_nal_mutation(&catalog, &key).err().unwrap().code,
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
        .push(json!({"id":nal_mutation::CAPABILITY_ID,"version":1}));
    let key = write_manifest(&root, &catalog, &manifest);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        accept_nal_mutation(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}
