use super::*;
use serde_json::json;
use std::sync::Arc;

pub(crate) fn fixture(bytes: &[u8]) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String) {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("writing");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("host"), bytes).unwrap();
    fs::write(package.join("manifest.json"), serde_json::to_vec(&json!({
        "formatVersion":2,"id":"writing_projects","version":"1.0.0","apiVersion":1,
        "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
        "provides":[{"id":"writing.import-preview","version":1}],
        "native":{"protocolVersion":1,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
    })).unwrap()).unwrap();
    let catalog = Arc::new(
        RuntimePluginCatalog::new(Some(root.path().into())).with_native_writing_preview(true),
    );
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    (root, catalog, key)
}

pub(crate) fn literature_fixture(
    bytes: &[u8],
) -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String) {
    let (root, catalog, _) = fixture(bytes);
    let directory = root.path().join("writing");
    fs::write(directory.join("manifest.json"), serde_json::to_vec(&json!({
        "formatVersion":2,"id":"literature_library","version":"0.1.0","apiVersion":1,
        "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
        "provides":[{"id":"literature.note-trace-decision","version":1}],
        "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
    })).unwrap()).unwrap();
    fs::create_dir(directory.join("data")).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in [
        "trace-read.v1.sql",
        "trace-expire.v1.sql",
        "trace-mark-sent.v1.sql",
    ] {
        fs::write(
            directory.join("data").join(name),
            fs::read(source.join(name)).unwrap(),
        )
        .unwrap();
    }
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    (root, catalog, key)
}

#[test]
fn literature_profile_has_separate_input_cwd_and_cannot_widen_writing_eligibility() {
    let (_root, catalog, key) = literature_fixture(b"#!/bin/sh\nexit 0\n");
    assert_eq!(
        stage(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
    assert_ne!(
        staged.directory.path(),
        staged.input_directory.as_ref().unwrap().path()
    );
    assert_eq!(
        staged.executable.file_name().unwrap(),
        "literature-library-host"
    );
    let (_root, writing, key) = fixture(b"host");
    assert_eq!(
        stage_for(&writing, &key, NativeProfile::NoteTraceDecision)
            .err()
            .unwrap()
            .plugin_id,
        "literature_library"
    );
}

#[test]
fn literature_execution_profile_rejects_frontend_versions_and_extra_dependencies() {
    let (root, catalog, _) = literature_fixture(b"host");
    let file = root.path().join("writing/manifest.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for mutate in [
        |value: &mut serde_json::Value| value["entry"] = json!("host"),
        |value: &mut serde_json::Value| value["native"]["protocolVersion"] = json!(1),
        |value: &mut serde_json::Value| value["provides"][0]["version"] = json!(2),
        |value: &mut serde_json::Value| {
            value["consumes"]["optional"] = json!([{"id":"other","version":1}])
        },
    ] {
        let mut value = original.clone();
        mutate(&mut value);
        fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert!(stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).is_err());
    }
}

#[test]
fn structured_validation_is_known_metadata_without_a_core_invocation_or_data_grant() {
    for combined in [false, true] {
        let (root, catalog, _) = literature_fixture(b"never executed");
        let file = root.path().join("writing/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        if combined {
            manifest = super::combined_tests::combined_manifest(&manifest);
            fs::write(root.path().join("writing/entry.js"), "export {};").unwrap();
            fs::write(root.path().join("writing/style.css"), "/* synthetic */").unwrap();
        }
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.structured-validation","version":1}));
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert!(accept(&catalog, &key, NativeProfile::NoteTraceDecision).is_ok());
        assert!(accept(&catalog, &key, NativeProfile::WritingPreview).is_err());
        for variant in 0..4 {
            let mut value = manifest.clone();
            let declarations = value["provides"].as_array_mut().unwrap();
            match variant {
                0 => declarations.last_mut().unwrap()["version"] = json!(2),
                1 => {
                    declarations.last_mut().unwrap()["id"] =
                        json!("literature.structured-validation.execute-sql");
                }
                2 => declarations.last_mut().unwrap()["sql"] = json!(true),
                _ => declarations.push(declarations.last().unwrap().clone()),
            }
            fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            assert_eq!(
                accept(&catalog, &key, NativeProfile::NoteTraceDecision)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn native_only_package_uses_the_same_content_snapshot_without_a_fake_frontend() {
    let (_root, catalog, key) = fixture(b"#!/bin/sh\nexit 0\n");
    let listing = catalog.discover().unwrap();
    assert!(listing.native_writing_preview);
    assert!(listing.packages[0].manifest.get("entry").is_none());
    let staged = stage(&catalog, &key).unwrap();
    assert_eq!(
        fs::read(&staged.executable).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(staged.directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(staged.executable)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
    }
}

#[test]
fn catalog_listing_serializes_the_switch_in_the_existing_camelcase_ipc_contract() {
    let (_root, catalog, _) = fixture(b"host");
    let enabled = serde_json::to_value(catalog.discover().unwrap()).unwrap();
    assert_eq!(enabled["nativeWritingPreview"], true);
    assert!(enabled.get("native_writing_preview").is_none());
    assert!(enabled["packages"].is_array());
    let disabled =
        serde_json::to_value(RuntimePluginCatalog::new(None).discover().unwrap()).unwrap();
    assert_eq!(disabled["nativeWritingPreview"], false);
    assert_eq!(disabled["packages"], json!([]));
}

#[test]
fn version_one_keeps_ignoring_unknown_native_metadata_without_executing_it() {
    let (root, catalog, _) = fixture(b"original frontend entry");
    fs::write(
        root.path().join("writing/manifest.json"),
        serde_json::to_vec(&json!({
            "formatVersion":1,"entry":"host","native":{"entry":"/ignored/unknown-metadata"}
        }))
        .unwrap(),
    )
    .unwrap();
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    assert_eq!(
        stage(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}

#[test]
fn changed_package_is_rejected_but_cannot_change_an_already_staged_instance() {
    let (root, catalog, key) = fixture(b"original");
    let staged = stage(&catalog, &key).unwrap();
    fs::write(root.path().join("writing/host"), b"changed").unwrap();
    assert_eq!(
        stage(&catalog, &key).err().unwrap().code,
        "runtime_plugin_resource_changed"
    );
    assert_eq!(fs::read(staged.executable).unwrap(), b"original");
}

#[test]
fn unsupported_profile_and_process_options_are_not_execution_authority() {
    let (root, catalog, _) = fixture(b"host");
    let file = root.path().join("writing/manifest.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for value in [
        {
            let mut v = original.clone();
            v["native"]["args"] = json!(["--anything"]);
            v
        },
        {
            let mut v = original.clone();
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"writing.projects","version":1}));
            v
        },
        {
            let mut v = original.clone();
            v["dependencies"]["required"] = json!([{"id":"other","apiVersion":1}]);
            v
        },
    ] {
        fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert_eq!(
            stage(&catalog, &key).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn research_export_requires_the_exact_workflow_capability_in_native_and_combined_packages() {
    for combined in [false, true] {
        let (root, catalog, _) = literature_fixture(b"never executed");
        let file = root.path().join("writing/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        if combined {
            manifest = super::combined_tests::combined_manifest(&manifest);
            fs::write(root.path().join("writing/entry.js"), "export {};").unwrap();
            fs::write(root.path().join("writing/style.css"), "/* synthetic */").unwrap();
        }
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.research-export","version":1}));
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert!(accept_research_export(&catalog, &key).is_err());
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.analysis-workflow","version":1}));
        fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert!(accept_research_export(&catalog, &key).is_ok());
        for variant in 0..3 {
            let mut value = manifest.clone();
            let capabilities = value["provides"].as_array_mut().unwrap();
            let export = capabilities
                .iter()
                .position(|entry| entry["id"] == "literature.research-export")
                .unwrap();
            match variant {
                0 => capabilities[export]["version"] = json!(2),
                1 => capabilities[export]["id"] = json!("literature.research-export.execute-sql"),
                _ => capabilities.push(capabilities[export].clone()),
            }
            fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            assert!(accept_research_export(&catalog, &key).is_err());
        }
    }
}

#[test]
fn browser_capture_requires_its_original_catalog_and_attachment_capabilities() {
    for combined in [false, true] {
        let (root, catalog, _) = literature_fixture(b"never executed");
        let file = root.path().join("writing/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        if combined {
            manifest = super::combined_tests::combined_manifest(&manifest);
            fs::write(root.path().join("writing/entry.js"), "export {};").unwrap();
            fs::write(root.path().join("writing/style.css"), "/* synthetic */").unwrap();
        }
        for capability in [
            "literature.browser-capture",
            "literature.catalog-read",
            "literature.catalog-save",
            "literature.attachments",
        ] {
            manifest["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":capability,"version":1}));
            fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            assert_eq!(
                accept_browser_capture(&catalog, &key).is_ok(),
                capability == "literature.attachments"
            );
        }
        for variant in 0..4 {
            let mut value = manifest.clone();
            let capabilities = value["provides"].as_array_mut().unwrap();
            let index = capabilities
                .iter()
                .position(|capability| capability["id"] == "literature.browser-capture")
                .unwrap();
            match variant {
                0 => capabilities[index]["version"] = json!(2),
                1 => capabilities[index]["id"] = json!("literature.browser-capture.execute-sql"),
                2 => capabilities.push(capabilities[index].clone()),
                _ => {
                    capabilities.retain(|capability| capability["id"] != "literature.catalog-save")
                }
            }
            fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            assert!(accept_browser_capture(&catalog, &key).is_err());
        }
    }
}
