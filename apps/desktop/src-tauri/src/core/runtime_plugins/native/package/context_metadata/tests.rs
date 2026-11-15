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
        package_tests::literature_fixture(b"synthetic metadata host; never run");
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
fn metadata_is_opt_in_and_declaration_alone_cannot_authorize_fixed_sql() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        let key = select(&root, &catalog, &manifest);
        assert!(accept(&catalog, &key, NativeProfile::NoteTraceDecision).is_ok());
        assert_eq!(
            accept_context_metadata(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        let key = select(&root, &catalog, &manifest);
        let package = accept_context_metadata(&catalog, &key).unwrap();
        assert_eq!(package.package_key, key);
        assert!(
            crate::runtime_plugins::context_metadata::ContextMetadataResources::load(
                &catalog, &package
            )
            .is_err()
        );
    }
}

#[test]
fn metadata_rejects_version_duplicate_and_arbitrary_sql_extensions() {
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        for variant in 0..4 {
            let mut candidate = manifest.clone();
            let values = candidate["provides"].as_array_mut().unwrap();
            match variant {
                0 => values.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    values.last_mut().unwrap()["id"] =
                        json!("literature.context-metadata.execute-sql")
                }
                2 => values.last_mut().unwrap()["sql"] = json!(true),
                _ => values.push(json!({"id":CAPABILITY_ID,"version":1})),
            }
            let key = select(&root, &catalog, &candidate);
            assert_eq!(
                accept_context_metadata(&catalog, &key).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn existing_full_footprints_remain_accepted_without_requiring_metadata() {
    let previous = [
        "literature.catalog-read",
        "literature.catalog-save",
        "literature.nal-query-collections",
        "literature.nal-bootstrap",
        "literature.nal-annotation",
        "literature.nal-mutation",
        "literature.duplicate-review",
        "literature.duplicate-undo",
        "literature.duplicate-merge",
        "literature.lifecycle",
        "literature.trash-cleanup",
        "literature.metadata",
        "literature.import-jobs",
        "literature.doi-preview",
        "literature.reference-records",
        "literature.attachments",
        "literature.pdf-import",
        "literature.concepts",
        "literature.reading-units",
        "literature.reading-evidence",
        "literature.structured-validation",
        "literature.unreviewed-selection",
        "literature.editable-template-validation",
        "literature.paper-context",
    ];
    for combined in [false, true] {
        let (root, catalog, mut manifest) = fixture(combined);
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .extend(previous.map(|id| json!({"id":id,"version":1})));
        assert_eq!(
            manifest["provides"].as_array().unwrap().len(),
            if combined { 27 } else { 25 }
        );
        let key = select(&root, &catalog, &manifest);
        assert!(accept(&catalog, &key, NativeProfile::NoteTraceDecision).is_ok());
        assert_eq!(
            accept_context_metadata(&catalog, &key).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":CAPABILITY_ID,"version":1}));
        let key = select(&root, &catalog, &manifest);
        assert!(accept_context_metadata(&catalog, &key).is_ok());
    }
}
