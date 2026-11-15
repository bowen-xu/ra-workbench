use super::*;

#[test]
fn bundled_root_is_shared_by_catalog_clones_and_keeps_the_first_resource_location() {
    let root = tempfile::tempdir().unwrap();
    package(root.path(), "bundled");
    let other = tempfile::tempdir().unwrap();
    let catalog = RuntimePluginCatalog::new(None);
    let original_clone = catalog.clone();
    assert!(catalog.discover().unwrap().packages.is_empty());
    catalog.attach_bundled_root(root.path().to_owned());
    let listing = original_clone.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    original_clone.attach_bundled_root(other.path().to_owned());
    assert_eq!(
        serde_json::to_value(catalog.discover().unwrap()).unwrap(),
        serde_json::to_value(&listing).unwrap()
    );
    assert_eq!(
        original_clone
            .resource(&listing.packages[0].package_key, "index.js")
            .unwrap()
            .bytes,
        b"import './assets/chunk.js';"
    );
}

#[test]
fn explicit_directory_keeps_precedence_over_bundled_resources_even_when_unavailable() {
    let bundled = tempfile::tempdir().unwrap();
    package(bundled.path(), "bundled");
    let explicit = tempfile::tempdir().unwrap();
    package(explicit.path(), "explicit");
    fs::write(
        explicit.path().join("explicit/index.js"),
        b"export const explicit = true;",
    )
    .unwrap();
    let catalog = RuntimePluginCatalog::new(Some(explicit.path().to_owned()));
    catalog.attach_bundled_root(bundled.path().to_owned());
    let expected = RuntimePluginCatalog::new(Some(explicit.path().to_owned()))
        .discover()
        .unwrap();
    assert_eq!(
        serde_json::to_value(catalog.discover().unwrap()).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    fs::remove_dir_all(explicit.path()).unwrap();
    assert_eq!(
        catalog.discover().unwrap_err().code,
        "runtime_plugin_resource_missing"
    );
    assert!(
        catalog
            .resource(&expected.packages[0].package_key, "index.js")
            .is_err()
    );
}

#[test]
fn rescanning_after_a_directory_rename_refreshes_the_same_content_url() {
    let (root, directory, catalog, key) = fixture();
    fs::rename(directory, root.path().join("renamed")).unwrap();

    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    assert_eq!(listing.packages[0].package_key, key);
    assert_eq!(catalog.snapshots.lock().unwrap().len(), 1);
    assert_eq!(
        catalog.resource(&key, "index.js").unwrap().bytes,
        b"import './assets/chunk.js';"
    );
    assert!(catalog.resource(&key, "assets/style.css").is_ok());
}

#[test]
fn rescanning_after_removing_an_identical_duplicate_keeps_the_content_url_usable() {
    let root = tempfile::tempdir().unwrap();
    let first = package(root.path(), "a");
    let second = package(root.path(), "b");
    let raw =
        serde_json::json!({"id":"note_manager","entry":"index.js","styles":["assets/style.css"]});
    manifest(&first, raw.clone());
    manifest(&second, raw);
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));

    // Discovery preserves both declarations for the SDK's duplicate-ID check;
    // neither the content key nor this native catalog resolves that conflict.
    let duplicate_listing = catalog.discover().unwrap();
    assert_eq!(duplicate_listing.packages.len(), 2);
    let key = duplicate_listing.packages[0].package_key.clone();
    assert_eq!(duplicate_listing.packages[1].package_key, key);
    fs::remove_dir_all(first).unwrap();

    let repaired_listing = catalog.discover().unwrap();
    assert_eq!(repaired_listing.packages.len(), 1);
    assert_eq!(repaired_listing.packages[0].package_key, key);
    assert_eq!(catalog.snapshots.lock().unwrap().len(), 1);
    assert_eq!(
        catalog.resource(&key, "index.js").unwrap().bytes,
        b"import './assets/chunk.js';"
    );
    assert!(catalog.resource(&key, "assets/style.css").is_ok());
}
