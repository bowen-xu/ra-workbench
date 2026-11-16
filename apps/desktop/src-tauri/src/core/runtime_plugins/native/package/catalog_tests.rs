use super::*;
use crate::runtime_plugins::{
    literature_catalog::data::CatalogResources, literature_trace::data::TraceResources,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(
    combined: bool,
    catalog_method: bool,
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
    if catalog_method {
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.catalog-read","version":1}));
    }
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in [
        "list-active-ids",
        "search-fts-ids",
        "search-year-ids",
        "item-main",
        "item-creators",
        "item-tags",
        "item-attachments",
        "item-project-ids",
        "item-metadata-sources",
    ] {
        let name = format!("catalog-{name}.v1.sql");
        fs::write(
            package.join("data").join(&name),
            fs::read(source.join(&name)).unwrap(),
        )
        .unwrap();
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

#[test]
fn original_trace_only_packages_remain_accepted_without_catalog_method_authority() {
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, false);
        let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        TraceResources::load(&catalog, &package).unwrap();
        let error = accept_catalog(&catalog, &key).err().unwrap();
        assert_eq!(error.code, "plugin_host_capability_unavailable");
        assert_eq!(error.plugin_id, "literature_library");
        assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    }
}

#[test]
fn native_only_trace_and_catalog_package_loads_both_real_fixed_resource_sets() {
    let (_root, catalog, key, _) = fixture(false, true);
    let package = accept_catalog(&catalog, &key).unwrap();
    assert!(package.manifest.get("entry").is_none());
    assert_eq!(package.manifest["provides"].as_array().unwrap().len(), 2);
    TraceResources::load(&catalog, &package).unwrap();
    CatalogResources::load(&catalog, &package).unwrap();
    accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
    assert!(staged.input_directory.is_some());
    assert_eq!(
        fs::read(staged.executable).unwrap(),
        b"synthetic host; never executed"
    );
}

#[test]
fn combined_four_capabilities_keep_the_same_native_profile_and_real_catalog_resources() {
    let (_root, catalog, key, mut manifest) = fixture(true, true);
    // The closed set is order-independent, not position-based authorization.
    manifest["provides"].as_array_mut().unwrap().reverse();
    let manifest: PackageManifest = serde_json::from_value(manifest).unwrap();
    validate_package(manifest, NativeProfile::NoteTraceDecision).unwrap();
    let package = accept_catalog(&catalog, &key).unwrap();
    assert_eq!(package.manifest["provides"].as_array().unwrap().len(), 4);
    TraceResources::load(&catalog, &package).unwrap();
    CatalogResources::load(&catalog, &package).unwrap();
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
}

#[test]
fn catalog_capability_set_rejects_duplicates_versions_unknowns_and_missing_trace() {
    type Mutation = fn(&mut Value);
    let mutations: [(&str, Mutation); 5] = [
        ("duplicate catalog", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"literature.catalog-read","version":1}));
        }),
        ("catalog version two", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
        }),
        ("unknown method", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                json!("literature.catalog-write");
        }),
        ("missing original trace", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|capability| capability["id"] != json!("literature.note-trace-decision"));
        }),
        ("extra capability authority", |v| {
            v["provides"].as_array_mut().unwrap().last_mut().unwrap()["methods"] =
                json!(["anything"]);
        }),
    ];
    for combined in [false, true] {
        let (root, catalog, _, original) = fixture(combined, true);
        for (name, mutate) in mutations {
            let mut manifest = original.clone();
            mutate(&mut manifest);
            let key = write_manifest(&root, &catalog, &manifest);
            let error = accept_catalog(&catalog, &key).err().unwrap();
            assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
            assert_eq!(error.plugin_id, "literature_library", "{name}");
            assert_eq!(error.package_key.as_deref(), Some(key.as_str()), "{name}");
        }
    }
}

#[test]
fn catalog_selection_cannot_adopt_the_writing_preview_profile() {
    let (_root, writing, key) = tests::fixture(b"synthetic writing host");
    accept(&writing, &key, NativeProfile::WritingPreview).unwrap();
    assert_eq!(
        accept_catalog(&writing, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
    for combined in [false, true] {
        let (_root, catalog, key, _) = fixture(combined, true);
        assert_eq!(
            stage(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn declaring_catalog_does_not_accept_changed_or_missing_immutable_sql_resources() {
    let (root, catalog, key, _) = fixture(true, true);
    let package = accept_catalog(&catalog, &key).unwrap();
    CatalogResources::load(&catalog, &package).unwrap();
    let sql = root.path().join("writing/data/catalog-item-main.v1.sql");
    fs::write(&sql, b"synthetic changed bytes").unwrap();
    assert!(CatalogResources::load(&catalog, &package).is_err());
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let package = accept_catalog(&catalog, &key).unwrap();
    assert!(CatalogResources::load(&catalog, &package).is_err());
    fs::remove_file(sql).unwrap();
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let package = accept_catalog(&catalog, &key).unwrap();
    assert!(CatalogResources::load(&catalog, &package).is_err());
}

mod save_tests {
    use super::*;
    use crate::runtime_plugins::literature_catalog::save::SaveResources;

    fn save_fixture(
        combined: bool,
    ) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
        let (root, catalog, _, mut manifest) = fixture(combined, true);
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.catalog-save","version":1}));
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for name in [
            "item-upsert",
            "creators-delete",
            "creator-insert",
            "item-creator-insert",
            "doi-delete",
            "doi-insert",
            "tags-delete",
            "tag-insert",
            "item-tag-insert",
            "provenance-upsert",
            "search-delete",
            "search-insert",
            "duplicate-doi-ids",
            "duplicate-title-ids",
            "duplicate-insert",
        ] {
            let name = format!("catalog-save-{name}.v1.sql");
            fs::copy(
                source.join(&name),
                root.path().join("writing/data").join(name),
            )
            .unwrap();
        }
        let key = write_manifest(&root, &catalog, &manifest);
        (root, catalog, key, manifest)
    }

    #[test]
    fn existing_read_packages_cannot_implicitly_acquire_save_authority() {
        for combined in [false, true] {
            let (_root, catalog, key, _) = fixture(combined, true);
            let package = accept_catalog(&catalog, &key).unwrap();
            CatalogResources::load(&catalog, &package).unwrap();
            let error = accept_catalog_save(&catalog, &key).err().unwrap();
            assert_eq!(error.code, "plugin_host_capability_unavailable");
            assert_eq!(error.plugin_id, "literature_library");
            assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
        }
    }

    #[test]
    fn explicit_native_three_and_combined_five_capabilities_load_the_real_save_resources() {
        for combined in [false, true] {
            let (root, catalog, _, mut manifest) = save_fixture(combined);
            manifest["provides"].as_array_mut().unwrap().reverse();
            let key = write_manifest(&root, &catalog, &manifest);
            let package = accept_catalog_save(&catalog, &key).unwrap();
            assert_eq!(
                package.manifest["provides"].as_array().unwrap().len(),
                if combined { 5 } else { 3 }
            );
            assert_eq!(
                SaveResources::load(&catalog, &package)
                    .unwrap()
                    .package_key(),
                key
            );
            CatalogResources::load(&catalog, &package).unwrap();
            TraceResources::load(&catalog, &package).unwrap();
            accept_catalog(&catalog, &key).unwrap();
            accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        }
    }

    #[test]
    fn save_rejects_missing_read_or_trace_duplicate_wrong_version_and_unknown_capability() {
        type Mutation = fn(&mut Value);
        let mutations: [(&str, Mutation); 5] = [
            ("missing catalog read", |v| {
                v["provides"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|capability| capability["id"] != json!("literature.catalog-read"));
            }),
            ("missing trace", |v| {
                v["provides"].as_array_mut().unwrap().retain(|capability| {
                    capability["id"] != json!("literature.note-trace-decision")
                });
            }),
            ("duplicate save", |v| {
                v["provides"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":"literature.catalog-save","version":1}));
            }),
            ("save version two", |v| {
                v["provides"].as_array_mut().unwrap().last_mut().unwrap()["version"] = json!(2);
            }),
            ("unknown capability", |v| {
                v["provides"].as_array_mut().unwrap().last_mut().unwrap()["id"] =
                    json!("literature.catalog-arbitrary-write");
            }),
        ];
        for combined in [false, true] {
            let (root, catalog, _, original) = save_fixture(combined);
            for (name, mutate) in mutations {
                let mut manifest = original.clone();
                mutate(&mut manifest);
                let key = write_manifest(&root, &catalog, &manifest);
                let error = accept_catalog_save(&catalog, &key).err().unwrap();
                assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
                assert_eq!(error.package_key.as_deref(), Some(key.as_str()), "{name}");
            }
        }
    }
}

#[test]
fn paper_trace_is_explicit_and_rejects_unknown_or_mismatched_capabilities() {
    for combined in [false, true] {
        let (root, catalog, key, mut manifest) = fixture(combined, false);
        assert_eq!(
            accept_paper_retrieval_trace(&catalog, &key)
                .err()
                .unwrap()
                .code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"].as_array_mut().unwrap().push(json!({
            "id":research_workbench_native_sdk::paper_retrieval::CAPABILITY_ID,"version":1
        }));
        let file = root.path().join("writing/manifest.json");
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages.remove(0).package_key;
        assert!(accept_paper_retrieval_trace(&catalog, &key).is_ok());
        for variant in 0..3 {
            let mut invalid = manifest.clone();
            let capabilities = invalid["provides"].as_array_mut().unwrap();
            match variant {
                0 => capabilities.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    capabilities.last_mut().unwrap()["id"] =
                        json!("literature.paper-retrieval-trace.execute-sql")
                }
                _ => capabilities.push(capabilities.last().unwrap().clone()),
            }
            fs::write(&file, serde_json::to_vec(&invalid).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages.remove(0).package_key;
            assert_eq!(
                accept_paper_retrieval_trace(&catalog, &key)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}
