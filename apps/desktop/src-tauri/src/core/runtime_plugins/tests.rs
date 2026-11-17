use super::*;
use serde_json::json;
use std::{fs, path::Path};
use tempfile::TempDir;

#[path = "location_tests.rs"]
mod location_tests;

#[path = "selection_tests.rs"]
mod selection_tests;

fn package(root: &Path, name: &str) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("assets")).unwrap();
    fs::write(directory.join("index.js"), b"import './assets/chunk.js';").unwrap();
    fs::write(
        directory.join("assets/chunk.js"),
        b"export const ready = true;",
    )
    .unwrap();
    fs::write(
        directory.join("assets/style.css"),
        b".notes { display: grid; }",
    )
    .unwrap();
    manifest(
        &directory,
        json!({"entry":"index.js","styles":["assets/style.css"]}),
    );
    directory
}

fn manifest(directory: &Path, value: serde_json::Value) {
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

fn fixture() -> (TempDir, PathBuf, RuntimePluginCatalog, String) {
    let root = tempfile::tempdir().unwrap();
    let directory = package(root.path(), "notes");
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    (root, directory, catalog, key)
}

#[test]
fn no_configuration_is_empty_without_a_default_directory() {
    let catalog = RuntimePluginCatalog::new(None);
    assert!(catalog.discover().unwrap().packages.is_empty());
    assert_eq!(
        catalog
            .resource(&"0".repeat(64), "index.js")
            .err()
            .unwrap()
            .code,
        "runtime_plugin_package_unknown"
    );
}

#[test]
fn errors_preserve_stable_codes_and_resource_http_statuses() {
    for (code, status) in [
        ("runtime_plugin_package_unknown", 404),
        ("runtime_plugin_resource_missing", 404),
        ("runtime_plugin_resource_changed", 409),
        ("runtime_plugin_path_invalid", 400),
        ("runtime_plugin_resource_unsupported", 400),
        ("runtime_plugin_resource_forbidden", 403),
        ("runtime_plugin_filesystem_unavailable", 500),
    ] {
        let error = RuntimePluginError::new(code, "安全原始原因")
            .package(&"a".repeat(64))
            .resource("assets/chunk.js");
        assert_eq!(error.http_status(), status);
        let serialized = serde_json::to_value(error).unwrap();
        assert_eq!(serialized["code"], code);
        assert_eq!(serialized["message"], "安全原始原因");
        assert_eq!(serialized["resource"], "assets/chunk.js");
        assert!(serialized["packageKey"].is_string());
    }
}

#[test]
fn configuration_requires_an_existing_absolute_directory_but_new_is_pure() {
    let relative = RuntimePluginCatalog::new(Some(PathBuf::from("plugins")));
    assert_eq!(
        relative.discover().unwrap_err().code,
        "runtime_plugin_root_invalid"
    );
    let root = tempfile::tempdir().unwrap();
    let missing = RuntimePluginCatalog::new(Some(root.path().join("missing")));
    assert_eq!(
        missing.discover().unwrap_err().code,
        "runtime_plugin_resource_missing"
    );
    fs::write(root.path().join("file"), "content").unwrap();
    let file = RuntimePluginCatalog::new(Some(root.path().join("file")));
    assert_eq!(
        file.discover().unwrap_err().code,
        "runtime_plugin_root_invalid"
    );
}

#[test]
fn ipc_shape_retains_raw_manifest_and_content_identity_without_absolute_paths() {
    let root = tempfile::tempdir().unwrap();
    let directory = package(root.path(), "notes");
    let raw = json!({"formatVersion":999,"id":"notes","entry":"index.js","styles":[],"unknown":{"keep":true}});
    manifest(&directory, raw.clone());
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages[0].manifest, raw);
    let key = &listing.packages[0].package_key;
    assert_eq!(key.len(), 64);
    assert!(
        key.bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let serialized = serde_json::to_string(&listing).unwrap();
    assert!(!serialized.contains(&root.path().to_string_lossy().to_string()));
    let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value["packages"][0]["packageKey"], *key);
    assert!(value["packages"][0].get("package_key").is_none());
}

#[test]
fn built_javascript_css_chunks_and_assets_are_read_from_the_snapshot() {
    let (_root, _directory, catalog, key) = fixture();
    let script = catalog.resource(&key, "index.js").unwrap();
    assert_eq!(script.bytes, b"import './assets/chunk.js';");
    assert_eq!(script.mime_type, "text/javascript; charset=utf-8");
    let chunk = catalog.resource(&key, "assets/chunk.js").unwrap();
    assert_eq!(chunk.bytes, b"export const ready = true;");
    let css = catalog.resource(&key, "assets/style.css").unwrap();
    assert_eq!(css.mime_type, "text/css; charset=utf-8");
    assert_eq!(
        catalog.resource(&key, "not-found.js").err().unwrap().code,
        "runtime_plugin_resource_unknown"
    );
}

#[test]
fn key_is_independent_of_absolute_root_and_package_directory_name() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    package(left.path(), "a");
    let right_package = package(right.path(), "different");
    let first = RuntimePluginCatalog::new(Some(left.path().to_owned()))
        .discover()
        .unwrap();
    let right_catalog = RuntimePluginCatalog::new(Some(right.path().to_owned()));
    let second = right_catalog.discover().unwrap();
    assert_eq!(
        first.packages[0].package_key,
        second.packages[0].package_key
    );
    fs::write(
        right_package.join("assets/chunk.js"),
        "export const ready = false;",
    )
    .unwrap();
    let changed = right_catalog.discover().unwrap();
    assert_ne!(
        first.packages[0].package_key,
        changed.packages[0].package_key
    );
}

#[test]
fn discovery_does_not_hide_duplicate_manifest_identities_from_the_sdk_resolver() {
    let root = tempfile::tempdir().unwrap();
    package(root.path(), "a");
    package(root.path(), "b");
    let listing = RuntimePluginCatalog::new(Some(root.path().to_owned()))
        .discover()
        .unwrap();
    assert_eq!(listing.packages.len(), 2);
    assert_eq!(
        listing.packages[0].package_key,
        listing.packages[1].package_key
    );
}

#[test]
fn manifest_must_be_json_and_have_a_constructed_entry() {
    let root = tempfile::tempdir().unwrap();
    let directory = package(root.path(), "notes");
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    fs::write(directory.join("manifest.json"), "{ broken").unwrap();
    assert_eq!(
        catalog.discover().unwrap_err().code,
        "runtime_plugin_manifest_invalid"
    );
    assert!(catalog.discover().unwrap_err().message.contains("notes"));
    for value in [
        json!({"styles":[]}),
        json!({"entry":null}),
        json!({"entry":4}),
    ] {
        manifest(&directory, value);
        assert_eq!(
            catalog.discover().unwrap_err().code,
            "runtime_plugin_entry_missing"
        );
    }
}

#[test]
fn declared_entry_and_styles_must_exist_as_regular_files() {
    let root = tempfile::tempdir().unwrap();
    let directory = package(root.path(), "notes");
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    for value in [
        json!({"entry":"missing.js","styles":[]}),
        json!({"entry":"assets","styles":[]}),
        json!({"entry":"index.js","styles":["missing.css"]}),
    ] {
        manifest(&directory, value);
        assert_eq!(
            catalog.discover().unwrap_err().code,
            "runtime_plugin_resource_missing"
        );
    }
}

#[test]
fn manifest_resource_paths_reject_urls_encodings_and_noncanonical_segments() {
    let root = tempfile::tempdir().unwrap();
    let directory = package(root.path(), "notes");
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    for path in [
        "/index.js",
        "../index.js",
        "a/../index.js",
        "a//index.js",
        "./index.js",
        "a\\index.js",
        "a%2Findex.js",
        "a%252findex.js",
        "index.js?x",
        "index.js#x",
        "https://example.com/index.js",
        "C:/index.js",
        "index.js/",
        "\u{7f}",
    ] {
        manifest(&directory, json!({"entry":path,"styles":[]}));
        assert_eq!(
            catalog.discover().unwrap_err().code,
            "runtime_plugin_path_invalid",
            "{path}"
        );
        manifest(&directory, json!({"entry":"index.js","styles":[path]}));
        assert_eq!(
            catalog.discover().unwrap_err().code,
            "runtime_plugin_path_invalid",
            "{path}"
        );
    }
}

#[test]
fn request_paths_cannot_escape_or_decode_into_the_filesystem() {
    let (_root, _directory, catalog, key) = fixture();
    for path in [
        "../manifest.json",
        "/manifest.json",
        "assets//chunk.js",
        "assets/../index.js",
        "assets%2Fchunk.js",
        "index.js?raw",
        "index.js#fragment",
        "assets\\chunk.js",
    ] {
        let error = catalog.resource(&key, path).err().unwrap();
        assert_eq!(error.code, "runtime_plugin_path_invalid", "{path}");
    }
    for invalid in ["notes", "../notes", &"A".repeat(64), &"0".repeat(63)] {
        assert_eq!(
            catalog.resource(invalid, "index.js").err().unwrap().code,
            "runtime_plugin_package_key_invalid"
        );
    }
}

#[test]
fn unknown_added_deleted_and_changed_resources_never_fall_back_to_html() {
    let (_root, directory, catalog, key) = fixture();
    fs::write(directory.join("added.js"), "export default 'new';").unwrap();
    assert_eq!(
        catalog.resource(&key, "added.js").err().unwrap().code,
        "runtime_plugin_resource_unknown"
    );
    fs::write(directory.join("index.js"), "export default 'changed';").unwrap();
    let changed = catalog.resource(&key, "index.js").err().unwrap();
    assert_eq!(changed.code, "runtime_plugin_resource_changed");
    assert_eq!(changed.package_key.as_deref(), Some(key.as_str()));
    assert_eq!(changed.resource.as_deref(), Some("index.js"));
    fs::remove_file(directory.join("assets/chunk.js")).unwrap();
    let missing = catalog.resource(&key, "assets/chunk.js").err().unwrap();
    assert_eq!(missing.code, "runtime_plugin_resource_missing");
    assert_eq!(missing.resource.as_deref(), Some("assets/chunk.js"));
}

#[test]
fn rescanning_keeps_old_live_package_urls_and_does_not_publish_partial_failure() {
    let (root, _directory, catalog, key) = fixture();
    let other = package(root.path(), "other");
    assert_eq!(catalog.discover().unwrap().packages.len(), 2);
    assert!(catalog.resource(&key, "index.js").is_ok());
    fs::write(other.join("manifest.json"), "{ broken").unwrap();
    assert!(catalog.discover().is_err());
    assert!(catalog.resource(&key, "assets/style.css").is_ok());
}

#[test]
fn too_many_package_directories_reject_the_scan_and_retain_existing_snapshots() {
    let (root, _directory, catalog, key) = fixture();
    for index in 0..64 {
        fs::create_dir(root.path().join(format!("extra-{index}"))).unwrap();
    }
    let error = catalog.discover().unwrap_err();
    assert_eq!(error.code, "runtime_plugin_catalog_limit");
    assert_eq!(error.http_status(), 413);
    assert_eq!(catalog.snapshots.lock().unwrap().len(), 1);
    assert!(catalog.resource(&key, "assets/style.css").is_ok());
}

#[test]
fn snapshot_limit_rejects_the_whole_new_scan_without_evicting_existing_urls() {
    let (root, directory, catalog, key) = fixture();
    for version in 1..127 {
        fs::write(
            directory.join("index.js"),
            format!("export const version = {version};"),
        )
        .unwrap();
        catalog.discover().unwrap();
    }
    assert_eq!(catalog.snapshots.lock().unwrap().len(), 127);
    let first = package(root.path(), "extra-a");
    manifest(
        &first,
        json!({"entry":"index.js","styles":[],"id":"extra_a"}),
    );
    let second = package(root.path(), "extra-b");
    manifest(
        &second,
        json!({"entry":"index.js","styles":[],"id":"extra_b"}),
    );
    let candidate = RuntimePluginCatalog::new(Some(root.path().to_owned()))
        .discover()
        .unwrap();
    let new_keys: Vec<_> = candidate
        .packages
        .iter()
        .filter(|package| {
            !catalog
                .snapshots
                .lock()
                .unwrap()
                .contains_key(&package.package_key)
        })
        .map(|package| package.package_key.clone())
        .collect();
    assert_eq!(new_keys.len(), 2);
    let error = catalog.discover().unwrap_err();
    assert_eq!(error.code, "runtime_plugin_catalog_limit");
    assert_eq!(error.http_status(), 413);
    assert_eq!(catalog.snapshots.lock().unwrap().len(), 127);
    for new_key in new_keys {
        assert_eq!(
            catalog.resource(&new_key, "index.js").err().unwrap().code,
            "runtime_plugin_package_unknown"
        );
    }
    assert!(catalog.resource(&key, "assets/style.css").is_ok());
    fs::remove_dir_all(&second).unwrap();
    catalog.discover().unwrap();
    assert_eq!(
        catalog.snapshots.lock().unwrap().len(),
        MAX_CATALOG_SNAPSHOTS
    );
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn discovery_rejects_package_directory_and_resource_symlinks() {
        let outside = tempfile::tempdir().unwrap();
        let external = package(outside.path(), "external");
        let root = tempfile::tempdir().unwrap();
        symlink(&external, root.path().join("linked-package")).unwrap();
        let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
        assert_eq!(
            catalog.discover().unwrap_err().code,
            "runtime_plugin_resource_unsupported"
        );
        fs::remove_file(root.path().join("linked-package")).unwrap();
        let directory = package(root.path(), "notes");
        for (target, link) in [
            (external.join("index.js"), directory.join("external.js")),
            (external.join("assets"), directory.join("external-dir")),
            (directory.join("index.js"), directory.join("internal.js")),
        ] {
            symlink(target, &link).unwrap();
            assert_eq!(
                catalog.discover().unwrap_err().code,
                "runtime_plugin_resource_unsupported"
            );
            fs::remove_file(link).unwrap();
        }
    }

    #[test]
    fn requests_reject_file_or_directory_replaced_with_an_external_symlink() {
        let (_root, directory, catalog, key) = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("other.js"), "do not return").unwrap();
        fs::remove_file(directory.join("index.js")).unwrap();
        symlink(outside.path().join("other.js"), directory.join("index.js")).unwrap();
        assert_eq!(
            catalog.resource(&key, "index.js").err().unwrap().code,
            "runtime_plugin_resource_unsupported"
        );
        fs::remove_dir_all(directory.join("assets")).unwrap();
        symlink(outside.path(), directory.join("assets")).unwrap();
        assert_eq!(
            catalog
                .resource(&key, "assets/chunk.js")
                .err()
                .unwrap()
                .code,
            "runtime_plugin_resource_unsupported"
        );
    }

    #[test]
    fn a_changed_configured_root_cannot_retarget_an_existing_snapshot() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let configuration = tempfile::tempdir().unwrap();
        package(first.path(), "notes");
        package(second.path(), "notes");
        let root = configuration.path().join("controlled");
        symlink(first.path(), &root).unwrap();
        let catalog = RuntimePluginCatalog::new(Some(root.clone()));
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        fs::remove_file(&root).unwrap();
        symlink(second.path(), &root).unwrap();
        assert_eq!(
            catalog.resource(&key, "index.js").err().unwrap().code,
            "runtime_plugin_resource_changed"
        );
    }
}
