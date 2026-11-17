use super::*;
use serde_json::json;

fn package(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("index.js"), "export function activate() {};").unwrap();
    fs::write(
        path.join("manifest.json"),
        serde_json::to_vec(&json!({
            "formatVersion":1, "id":name, "version":"0.1.0", "apiVersion":1,
            "entry":"index.js", "styles":[], "uiRuntime":"react@19.2.8/react-dom@19.2.8",
            "dependencies":{"required":[], "optional":[]}, "provides":[],
            "consumes":{"required":[], "optional":[]}
        }))
        .unwrap(),
    )
    .unwrap();
    path
}
fn catalog(base: Option<PathBuf>, managed: &Path) -> RuntimePluginCatalog {
    let catalog = RuntimePluginCatalog::new(base);
    catalog.attach_managed_root(managed.to_owned());
    catalog
}

#[test]
fn standalone_install_enable_restart_and_uninstall_keep_admitted_resources() {
    let source = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    let original = package(source.path(), "notes");
    let first = catalog(None, managed.path());
    let revision = first.managed_plugins().unwrap().revision;
    let installed = first
        .install_managed_plugin(&original.join("manifest.json"), &revision)
        .unwrap();
    fs::remove_dir_all(&original).unwrap();
    assert!(first.discover().unwrap().packages.is_empty());
    let list = first.managed_plugins().unwrap();
    assert!(!list.plugins[0].enabled);
    first
        .set_managed_plugin("notes", true, false, &list.revision)
        .unwrap();
    assert!(first.discover().unwrap().packages.is_empty());
    let second = catalog(None, managed.path());
    assert_eq!(
        second.discover().unwrap().packages[0].package_key,
        installed.package_key
    );
    let revision = second.managed_plugins().unwrap().revision;
    second
        .set_managed_plugin("notes", false, true, &revision)
        .unwrap();
    assert!(second.managed_plugins().unwrap().plugins[0].removed);
    let third = catalog(None, managed.path());
    assert!(third.discover().unwrap().packages.is_empty());
    assert!(third.managed_plugins().unwrap().plugins.is_empty());
    assert!(second.resource(&installed.package_key, "index.js").is_ok());
    drop(second);
    drop(first);
    let fourth = catalog(None, managed.path());
    assert!(fourth.discover().unwrap().packages.is_empty());
    assert!(
        !managed
            .path()
            .join("packages")
            .join(installed.package_key)
            .exists()
    );
}

#[test]
fn bundled_disable_reenable_and_uninstall_tombstone_survive_reopen() {
    let base = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    package(base.path(), "notes");
    let open = || catalog(Some(base.path().to_owned()), managed.path());
    let first = open();
    let revision = first.managed_plugins().unwrap().revision;
    first
        .set_managed_plugin("notes", false, false, &revision)
        .unwrap();
    assert_eq!(first.discover().unwrap().packages.len(), 1);
    let second = open();
    assert!(second.discover().unwrap().packages.is_empty());
    let list = second.managed_plugins().unwrap();
    assert!(!list.plugins[0].enabled);
    second
        .set_managed_plugin("notes", true, false, &list.revision)
        .unwrap();
    let third = open();
    assert_eq!(third.discover().unwrap().packages.len(), 1);
    let revision = third.managed_plugins().unwrap().revision;
    third
        .set_managed_plugin("notes", false, true, &revision)
        .unwrap();
    let fourth = open();
    assert!(fourth.managed_plugins().unwrap().plugins.is_empty());
    assert!(base.path().join("notes/manifest.json").exists());
}

#[test]
fn stale_revision_rejects_write_and_does_not_replace_selection() {
    let base = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    package(base.path(), "notes");
    let catalog = catalog(Some(base.path().to_owned()), managed.path());
    let revision = catalog.managed_plugins().unwrap().revision;
    catalog
        .set_managed_plugin("notes", false, false, &revision)
        .unwrap();
    assert_eq!(
        catalog
            .set_managed_plugin("notes", true, false, &revision)
            .unwrap_err()
            .code,
        "plugin_registry_conflict"
    );
    assert!(!catalog.managed_plugins().unwrap().plugins[0].enabled);
}

#[test]
fn zip_import_uses_same_catalog_and_rejects_traversal_without_registration() {
    use std::io::Write;
    let source = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    let original = package(source.path(), "notes");
    let zip = source.path().join("notes.zip");
    let mut archive = zip::ZipWriter::new(fs::File::create(&zip).unwrap());
    for file in ["manifest.json", "index.js"] {
        archive
            .start_file(file, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(&fs::read(original.join(file)).unwrap())
            .unwrap();
    }
    archive.finish().unwrap();
    let catalog = catalog(None, managed.path());
    let revision = catalog.managed_plugins().unwrap().revision;
    catalog.install_managed_plugin(&zip, &revision).unwrap();
    let mut archive = zip::ZipWriter::new(fs::File::create(&zip).unwrap());
    archive
        .start_file("../escape.js", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"bad").unwrap();
    archive.finish().unwrap();
    let before = catalog.managed_plugins().unwrap();
    assert_eq!(
        catalog
            .install_managed_plugin(&zip, &before.revision)
            .unwrap_err()
            .code,
        "runtime_plugin_path_invalid"
    );
    assert_eq!(catalog.managed_plugins().unwrap().revision, before.revision);
    assert!(!managed.path().join("escape.js").exists());
}

#[cfg(unix)]
#[test]
fn symbolic_link_source_is_rejected_without_installing() {
    use std::os::unix::fs::symlink;
    let source = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    let original = package(source.path(), "notes");
    symlink(
        source.path().join("notes/index.js"),
        original.join("link.js"),
    )
    .unwrap();
    let catalog = catalog(None, managed.path());
    let revision = catalog.managed_plugins().unwrap().revision;
    assert_eq!(
        catalog
            .install_managed_plugin(&original, &revision)
            .unwrap_err()
            .code,
        "runtime_plugin_resource_unsupported"
    );
    assert!(catalog.managed_plugins().unwrap().plugins.is_empty());
}

#[test]
fn missing_installed_bytes_can_be_diagnosed_and_uninstalled() {
    let source = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    let original = package(source.path(), "notes");
    let first = catalog(None, managed.path());
    let revision = first.managed_plugins().unwrap().revision;
    let installed = first.install_managed_plugin(&original, &revision).unwrap();
    fs::remove_file(
        managed
            .path()
            .join("packages")
            .join(installed.package_key)
            .join("index.js"),
    )
    .unwrap();
    let list = first.managed_plugins().unwrap();
    assert!(list.plugins[0].diagnostic.is_some());
    first
        .set_managed_plugin("notes", false, true, &list.revision)
        .unwrap();
    assert!(first.managed_plugins().unwrap().plugins.is_empty());
}

#[test]
fn corrupt_registry_is_preserved_and_is_not_treated_as_an_empty_installation() {
    let managed = tempfile::tempdir().unwrap();
    let path = managed.path().join("registry.json");
    fs::write(&path, "").unwrap();
    let first = catalog(None, managed.path());
    assert_eq!(
        first.managed_plugins().err().unwrap().code,
        "plugin_registry_invalid"
    );
    assert_eq!(fs::read(&path).unwrap(), b"");
}
