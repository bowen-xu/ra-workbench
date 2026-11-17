use super::*;
use std::sync::{Barrier, atomic::Ordering};

#[test]
fn selected_package_reuses_discovery_and_keeps_resource_hash_checks() {
    let (root, directory, catalog, key) = fixture();
    // An unrelated broken installation cannot force a rescan on business calls.
    fs::create_dir(root.path().join("unrelated")).unwrap();
    for _ in 0..20 {
        assert_eq!(catalog.package(&key).unwrap().package_key, key);
        catalog.resource(&key, "index.js").unwrap();
    }
    assert_eq!(catalog.discovery_count.load(Ordering::Relaxed), 1);
    let path = directory.join("index.js");
    let original = fs::read(&path).unwrap();
    // Equal size also rejects; metadata is never used as an integrity proof.
    fs::write(&path, vec![b'x'; original.len()]).unwrap();
    assert_eq!(
        catalog.resource(&key, "index.js").err().unwrap().code,
        "runtime_plugin_resource_changed"
    );
    assert!(catalog.discover().is_err());
}

#[test]
fn concurrent_first_acceptance_discovers_once_across_catalog_clones() {
    let (root, _, _, key) = fixture();
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        let mut callers = Vec::new();
        for _ in 0..8 {
            let catalog = catalog.clone();
            let barrier = Arc::clone(&barrier);
            let key = key.clone();
            callers.push(scope.spawn(move || {
                barrier.wait();
                catalog.package(&key).unwrap().package_key
            }));
        }
        for caller in callers {
            assert_eq!(caller.join().unwrap(), key);
        }
    });
    assert_eq!(catalog.discovery_count.load(Ordering::Relaxed), 1);
}

#[test]
fn changed_manifest_is_rejected_and_explicit_discovery_creates_a_new_identity() {
    let (_root, directory, catalog, key) = fixture();
    let file = directory.join("manifest.json");
    let mut bytes = fs::read(&file).unwrap();
    bytes.push(b' ');
    fs::write(file, bytes).unwrap();
    let error = catalog.package(&key).unwrap_err();
    assert_eq!(error.code, "runtime_plugin_resource_changed");
    assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    assert_eq!(catalog.discovery_count.load(Ordering::Relaxed), 1);
    let replacement = catalog.discover().unwrap().packages.remove(0);
    assert_ne!(replacement.package_key, key);
    assert!(catalog.package(&replacement.package_key).is_ok());
    assert_eq!(
        catalog.package(&key).unwrap_err().code,
        "runtime_plugin_resource_changed"
    );
}

#[test]
fn selected_package_keeps_snapshot_bound_and_unknown_resources_closed() {
    let (_root, directory, catalog, key) = fixture();
    fs::write(directory.join("added.js"), "export {};").unwrap();
    assert!(catalog.package(&key).is_ok());
    assert_eq!(
        catalog.resource(&key, "added.js").err().unwrap().code,
        "runtime_plugin_resource_unknown"
    );
    fs::remove_file(directory.join("manifest.json")).unwrap();
    assert_eq!(
        catalog.package(&key).unwrap_err().code,
        "runtime_plugin_resource_missing"
    );
}

#[test]
#[ignore = "synthetic filesystem timing comparison; run explicitly with --nocapture"]
fn repeated_package_acceptance_timing() {
    let (root, directory, _, _) = fixture();
    for index in 0..1024 {
        fs::write(
            directory.join(format!("assets/{index}.js")),
            vec![b' '; 1024],
        )
        .unwrap();
    }
    fs::write(directory.join("assets/large.bin"), vec![0; 8 * 1024 * 1024]).unwrap();
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let key = catalog.discover().unwrap().packages.remove(0).package_key;
    const CALLS: usize = 20;
    let before = std::time::Instant::now();
    for _ in 0..CALLS {
        assert_eq!(catalog.discover().unwrap().packages[0].package_key, key);
    }
    let rescanning = before.elapsed();
    let before = std::time::Instant::now();
    for _ in 0..CALLS {
        assert_eq!(catalog.package(&key).unwrap().package_key, key);
        catalog.resource(&key, "index.js").unwrap();
    }
    let selected = before.elapsed();
    assert_eq!(catalog.discovery_count.load(Ordering::Relaxed), CALLS + 1);
    eprintln!(
        "synthetic package, 1029 files / about 9 MiB, {CALLS} calls: full discovery {rescanning:?}, selected manifest + resource {selected:?}"
    );
}
