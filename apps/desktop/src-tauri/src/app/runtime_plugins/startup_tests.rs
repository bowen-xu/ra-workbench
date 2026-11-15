//! No window, host process, SQL transaction or GC worker is started here.
use super::*;
use crate::runtime_plugins::{
    RuntimePluginPackage,
    native::{NativeHostError, NativeProfile, NativeResult},
};
use research_workbench_native_sdk::trash_cleanup::{CAPABILITY_ID, CAPABILITY_VERSION};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Mutex};
use tokio::{
    sync::oneshot,
    time::{Duration, timeout},
};

const WAIT: Duration = Duration::from_secs(8);

#[test]
fn each_native_profile_selects_only_its_exact_unique_deployed_package() {
    let packages = [
        RuntimePluginPackage {
            package_key: "a".repeat(64),
            manifest: json!({"id":"literature_library"}),
        },
        RuntimePluginPackage {
            package_key: "b".repeat(64),
            manifest: json!({"id":"writing_projects"}),
        },
        RuntimePluginPackage {
            package_key: "c".repeat(64),
            manifest: json!({"id":"note_manager"}),
        },
    ];
    for (profile, key) in [
        (NativeProfile::NoteTraceDecision, "a"),
        (NativeProfile::WritingProjects, "b"),
    ] {
        assert_eq!(
            select_native_package(&packages, profile).unwrap(),
            Some(key.repeat(64))
        );
        assert_eq!(
            select_native_package(&packages[2..], profile).unwrap(),
            None
        );
        let duplicated = [
            RuntimePluginPackage {
                package_key: "d".repeat(64),
                manifest: json!({"id":profile.plugin_id()}),
            },
            RuntimePluginPackage {
                package_key: "e".repeat(64),
                manifest: json!({"id":profile.plugin_id()}),
            },
        ];
        let error = select_native_package(&duplicated, profile).unwrap_err();
        assert_eq!(error.code, "plugin_host_package_ambiguous");
        assert_eq!(error.plugin_id, profile.plugin_id());
    }
}

async fn unexpected_purge(_: String) -> NativeResult<()> {
    panic!("Unselected or rejected startup providers must not invoke purge")
}

struct Fixture {
    root: tempfile::TempDir,
    catalog: Arc<RuntimePluginCatalog>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("literature");
        fs::create_dir_all(directory.join("data")).unwrap();
        fs::write(
            directory.join("host"),
            b"synthetic startup host; never executed",
        )
        .unwrap();
        fs::write(directory.join("manifest.json"),serde_json::to_vec(&json!({
            "formatVersion":2,"id":"literature_library","version":"0.1.0","apiVersion":1,
            "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
            "provides":[{"id":"literature.note-trace-decision","version":1},{"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}],
            "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
        })).unwrap()).unwrap();
        Self {
            catalog: Arc::new(RuntimePluginCatalog::new(Some(root.path().to_owned()))),
            root,
        }
    }

    fn manifest(&self, change: impl FnOnce(&mut Value)) {
        let path = self.root.path().join("literature/manifest.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn key(&self) -> String {
        self.catalog
            .discover()
            .unwrap()
            .packages
            .remove(0)
            .package_key
    }

    fn copy_resources(&self) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../plugins/literature_library/native/data");
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "sql") {
                fs::copy(
                    entry.path(),
                    self.root
                        .path()
                        .join("literature/data")
                        .join(entry.file_name()),
                )
                .unwrap();
            }
        }
    }
}

#[tokio::test]
async fn absent_literature_is_a_successful_no_provider_for_core_notes_and_writing() {
    let root = tempfile::tempdir().unwrap();
    for id in ["note_manager", "writing_projects"] {
        let directory = root.path().join(id);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("entry.js"), b"throw Error('never executed')").unwrap();
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&json!({"id":id,"entry":"entry.js","styles":[]})).unwrap(),
        )
        .unwrap();
    }
    for catalog in [
        Arc::new(RuntimePluginCatalog::new(None)),
        Arc::new(RuntimePluginCatalog::new(Some(root.path().to_owned()))),
    ] {
        let result = run_startup_purge(catalog, unexpected_purge).await.unwrap();
        assert_eq!(result, StartupTrashCleanup::NoProvider);
    }
}

#[tokio::test]
async fn unique_provider_uses_its_exact_content_key() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let selected = key.clone();
    let result = run_startup_purge(Arc::clone(&fixture.catalog), move |actual| async move {
        assert_eq!(actual, selected);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(result, StartupTrashCleanup::Completed { package_key: key });
}

#[tokio::test]
async fn duplicate_including_identical_package_keys_is_never_a_first_or_latest_selection() {
    let fixture = Fixture::new();
    let original = fixture.root.path().join("literature");
    let duplicate = fixture.root.path().join("literature-copy");
    fs::create_dir_all(duplicate.join("data")).unwrap();
    for name in ["manifest.json", "host"] {
        fs::copy(original.join(name), duplicate.join(name)).unwrap();
    }
    let listing = fixture.catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 2);
    assert_eq!(
        listing.packages[0].package_key,
        listing.packages[1].package_key
    );
    let error = run_startup_purge(Arc::clone(&fixture.catalog), unexpected_purge)
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_package_ambiguous");
    assert_eq!(error.plugin_id, "literature_library");
    assert!(
        error.package_key.is_none() && error.instance_id.is_none() && error.request_id.is_none()
    );
    // A different version/content is equally ambiguous; never prefer the newer one.
    fixture.manifest(|v| v["version"] = json!("9.0.0"));
    assert_eq!(
        run_startup_purge(Arc::clone(&fixture.catalog), unexpected_purge)
            .await
            .unwrap_err()
            .code,
        "plugin_host_package_ambiguous"
    );
}

#[tokio::test]
async fn missing_capability_and_invalid_profile_fail_before_any_purge_effect() {
    type ManifestCase = (fn(&mut Value), &'static str);
    let cases: [ManifestCase; 3] = [
        (
            |v| {
                v["provides"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|c| c["id"] != CAPABILITY_ID);
            },
            "plugin_host_capability_unavailable",
        ),
        (
            |v| v["native"]["protocolVersion"] = json!(1),
            "plugin_host_protocol_incompatible",
        ),
        (
            |v| v["dependencies"]["required"] = json!([{"id":"other","version":1}]),
            "plugin_host_profile_unsupported",
        ),
    ];
    for (mutation, expected_code) in cases {
        let fixture = Fixture::new();
        fixture.manifest(mutation);
        let key = fixture.key();
        let error = run_startup_purge(Arc::clone(&fixture.catalog), unexpected_purge)
            .await
            .unwrap_err();
        assert_eq!(error.code, expected_code);
        assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
        assert!(error.instance_id.is_none() && error.request_id.is_none());
    }
}

#[tokio::test]
async fn changed_fixed_resource_is_rejected_by_the_actual_cleanup_loader() {
    use crate::runtime_plugins::{
        literature_trash_cleanup::TrashCleanupResources, native::accept_trash_cleanup,
    };
    let fixture = Fixture::new();
    fixture.copy_resources();
    let catalog = Arc::clone(&fixture.catalog);
    let directory = fixture.root.path().join("literature");
    let error = run_startup_purge(Arc::clone(&catalog), move |key| async move {
        let package = accept_trash_cleanup(&catalog, &key)?;
        let resource = directory.join("data/trash-cleanup-root.v1.sql");
        let mut bytes = fs::read(&resource).unwrap();
        bytes.push(b'\n');
        fs::write(resource, bytes).unwrap();
        TrashCleanupResources::load(&catalog, &package).map_err(startup_catalog_error)?;
        Ok(())
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, "runtime_plugin_resource_changed");
    assert!(error.instance_id.is_none() && error.request_id.is_none());
}

#[tokio::test]
async fn caller_continues_to_gc_only_after_purge_settles_and_logs_failures_first() {
    for fail in [false, true] {
        let fixture = Fixture::new();
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let work_events = Arc::clone(&events);
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let run = tokio::spawn(async move {
            let purge_events = Arc::clone(&work_events);
            let result = run_startup_purge(fixture.catalog, move |_| async move {
                purge_events.lock().unwrap().push("purge-start".into());
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
                purge_events.lock().unwrap().push("purge-settled".into());
                if fail {
                    Err(NativeHostError::for_profile(
                        NativeProfile::NoteTraceDecision,
                        "storage_error",
                        "safe original cause",
                    ))
                } else {
                    Ok(())
                }
            })
            .await;
            if let Err(error) = &result {
                work_events
                    .lock()
                    .unwrap()
                    .push(format!("log:{}:{}", error.code, error.message));
            }
            work_events.lock().unwrap().push("gc-schedule".into());
            result
        });
        timeout(WAIT, entered_rx).await.unwrap().unwrap();
        assert_eq!(*events.lock().unwrap(), ["purge-start"]);
        assert!(!run.is_finished());
        release_tx.send(()).unwrap();
        let result = timeout(WAIT, run).await.unwrap().unwrap();
        let expected = if fail {
            vec![
                "purge-start",
                "purge-settled",
                "log:storage_error:safe original cause",
                "gc-schedule",
            ]
        } else {
            vec!["purge-start", "purge-settled", "gc-schedule"]
        };
        assert_eq!(*events.lock().unwrap(), expected);
        assert_eq!(result.is_err(), fail);
    }
}

#[test]
fn provider_identity_is_exact_without_id_or_version_normalization() {
    let packages = [RuntimePluginPackage {
        package_key: "a".repeat(64),
        manifest: json!({"id":" literature_library","version":"9.0.0"}),
    }];
    assert_eq!(select_startup_package(&packages).unwrap(), None);
}
