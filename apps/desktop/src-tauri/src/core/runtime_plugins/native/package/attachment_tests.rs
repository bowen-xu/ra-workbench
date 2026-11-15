use super::*;
use serde_json::{Value, json};

fn manifest(
    combined: bool,
) -> (
    tempfile::TempDir,
    std::sync::Arc<RuntimePluginCatalog>,
    Value,
) {
    let (root, catalog, _) =
        tests::literature_fixture(b"synthetic attachment host, never executed");
    let mut value: Value =
        serde_json::from_slice(&fs::read(root.path().join("writing/manifest.json")).unwrap())
            .unwrap();
    if combined {
        value = combined_tests::combined_manifest(&value);
        fs::write(root.path().join("writing/entry.js"), "export {};").unwrap();
        fs::write(root.path().join("writing/style.css"), "/* synthetic */").unwrap();
    }
    (root, catalog, value)
}

fn key(root: &tempfile::TempDir, catalog: &RuntimePluginCatalog, value: &Value) -> String {
    fs::write(
        root.path().join("writing/manifest.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
    catalog.discover().unwrap().packages.remove(0).package_key
}

#[test]
fn attachment_methods_require_their_own_capability_and_exact_resources() {
    for combined in [false, true] {
        let (root, catalog, mut value) = manifest(combined);
        let selected = key(&root, &catalog, &value);
        assert_eq!(
            accept_attachments(&catalog, &selected).err().unwrap().code,
            "plugin_host_capability_unavailable"
        );
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.attachments","version":1}));
        let selected = key(&root, &catalog, &value);
        let package = accept_attachments(&catalog, &selected).unwrap();
        assert_eq!(package.package_key, selected);
        assert!(
            crate::runtime_plugins::literature_attachments::AttachmentResources::load(
                &catalog, &package
            )
            .is_err()
        );
    }
}

#[test]
fn attachment_declaration_does_not_authorize_other_versions_duplicates_or_arbitrary_sql() {
    for combined in [false, true] {
        let (root, catalog, mut original) = manifest(combined);
        original["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.attachments","version":1}));
        for variant in 0..4 {
            let mut value = original.clone();
            let caps = value["provides"].as_array_mut().unwrap();
            match variant {
                0 => caps.last_mut().unwrap()["version"] = json!(2),
                1 => caps.last_mut().unwrap()["id"] = json!("literature.attachments.execute-sql"),
                2 => caps.last_mut().unwrap()["sql"] = json!(true),
                _ => caps.push(json!({"id":"literature.attachments","version":1})),
            }
            let selected = key(&root, &catalog, &value);
            assert_eq!(
                accept_attachments(&catalog, &selected).err().unwrap().code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn pdf_import_requires_all_existing_save_attachment_and_shared_job_capabilities() {
    for combined in [false, true] {
        let (root, catalog, mut value) = manifest(combined);
        let baseline = value["provides"].as_array().unwrap().clone();
        let required = [
            "literature.attachments",
            "literature.catalog-read",
            "literature.catalog-save",
            "literature.import-jobs",
            "literature.pdf-import",
        ];
        for omitted in 0..required.len() {
            value["provides"] = serde_json::to_value(&baseline).unwrap();
            for (index, id) in required.iter().enumerate() {
                if index != omitted {
                    value["provides"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"id":id,"version":1}));
                }
            }
            let selected = key(&root, &catalog, &value);
            assert!(accept_pdf_import(&catalog, &selected).is_err());
        }
        value["provides"] = serde_json::to_value(&baseline).unwrap();
        for id in required {
            value["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"version":1}));
        }
        let selected = key(&root, &catalog, &value);
        assert_eq!(
            accept_pdf_import(&catalog, &selected).unwrap().package_key,
            selected
        );
    }
}
