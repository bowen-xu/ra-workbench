use super::*;
use std::fs;

#[test]
fn lazy_selection_observes_later_packages_and_refuses_explicit_missing_or_ambiguous() {
    let root = tempfile::tempdir().unwrap();
    let catalog =
        RuntimePluginCatalog::new(Some(root.path().to_owned())).with_native_writing_preview(true);
    assert!(select(&catalog, None).unwrap().is_none());
    let (source, _catalog, _) = fixture::package();
    let copy = |name: &str| {
        let target = root.path().join(name);
        fs::create_dir_all(target.join("data")).unwrap();
        for file in ["host", "manifest.json"] {
            fs::copy(source.path().join("writing").join(file), target.join(file)).unwrap();
        }
        for file in fs::read_dir(source.path().join("writing/data")).unwrap() {
            let file = file.unwrap();
            fs::copy(file.path(), target.join("data").join(file.file_name())).unwrap();
        }
    };
    copy("one");
    let accepted = select(&catalog, None).unwrap().unwrap();
    let key = accepted.package_key().to_owned();
    assert_eq!(
        select(&catalog, Some(&key)).unwrap().unwrap().package_key(),
        key
    );
    for explicit in ["", "missing-key"] {
        assert!(
            select(&catalog, Some(explicit)).is_err(),
            "explicit selection cannot fall back"
        );
    }
    copy("two");
    // Give the second provider its own actual content key; identical snapshots
    // deliberately deduplicate in the existing discovery mechanism.
    fs::write(
        root.path().join("two/host"),
        "#!/bin/sh\n# distinct provider\nexit 0\n",
    )
    .unwrap();
    assert_eq!(
        select(&catalog, None).err().unwrap().code,
        "plugin_host_package_ambiguous"
    );
    assert_eq!(
        select(&catalog, Some(&key)).unwrap().unwrap().package_key(),
        key
    );
    fs::remove_dir_all(root.path().join("one")).unwrap();
    assert!(
        select(&catalog, Some(&key)).is_err(),
        "removed exact provider cannot bind the surviving provider"
    );
    let second = select(&catalog, None).unwrap().unwrap();
    assert_ne!(second.package_key(), key);
}

#[tokio::test]
async fn another_selected_snapshot_never_replaces_live_instance() {
    let fixture = Fixture::new(true).await;
    let host = fixture.package.path().join("writing/host");
    fs::write(
        host,
        "#!/bin/sh\n# replacement cannot reuse live identity\nexit 0\n",
    )
    .unwrap();
    let key = fixture.service.catalog.discover().unwrap().packages[0]
        .package_key
        .clone();
    assert_ne!(key, fixture.key);
    let error = fixture
        .service
        .structured_legacy(
            Arc::clone(&fixture.db),
            Some(key.clone()),
            Fixture::legacy(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_restart_required");
    assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    assert_eq!(fixture.imported(), 0);
    fixture.assert_no_catalog_rpc();
    fixture.service.shutdown().await.unwrap();
    fixture.assert_closed().await;
    assert_eq!(fixture.cold_reopen(), (0, String::new()));
}

#[tokio::test]
async fn actual_initialize_failure_keeps_its_real_request_identity() {
    let data = Fixture::new(false).await;
    let (root, catalog, _) = fixture::package();
    fs::write(
        root.path().join("writing/host"),
        r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"wrong-capability","version":1}}\n' "$identity"
"#,
    )
    .unwrap();
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let service = Arc::new(LiteratureTraceService::new(catalog));
    let error = timeout(
        WAIT,
        service.structured_sources(Arc::clone(&data.db), Some(key.clone()), "missing".into()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    assert!(
        error.instance_id.is_some() && error.request_id.is_some(),
        "the failed initialize really sent a request"
    );
    assert_ne!(
        error.code, "not_found",
        "Core SQL must not run after failed handshake"
    );
    service.shutdown().await.unwrap();
    assert!(service.host.lock().await.is_none());
    assert!(!service.cleanup_failed.load(Ordering::Acquire));
    drop(service);
    data.service.shutdown().await.unwrap();
    data.assert_closed().await;
    assert_eq!(data.cold_reopen(), (0, String::new()));
}
