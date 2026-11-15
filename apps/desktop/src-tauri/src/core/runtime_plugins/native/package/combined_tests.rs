use super::*;
use crate::runtime_plugins::literature_trace::data::TraceResources;
#[cfg(unix)]
use crate::runtime_plugins::native::transport::NativeHost;
#[cfg(unix)]
use research_workbench_native_sdk::{
    literature_trace_protocol::{SnapshotInput, TraceDecision},
    retrieval_trace::TraceRejection,
};
use serde_json::{Value, json};
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;

pub(super) fn combined_manifest(native_only: &Value) -> Value {
    let mut value = native_only.clone();
    value["version"] = json!("1.0.0");
    value["entry"] = json!("entry.js");
    value["styles"] = json!(["style.css"]);
    value["uiRuntime"] = json!("react@19.2.8/react-dom@19.2.8");
    value["provides"] = json!([
        {"id":"literature.catalog","version":1},
        {"id":"literature.citations","version":1},
        {"id":"literature.note-trace-decision","version":1}
    ]);
    let required = [
        "core.workbench",
        "core.workbench.source-workspace-host",
        "core.sdk.literature-shared",
        "core.notes",
        "core.editor",
        "core.hbs",
        "core.agent",
        "core.agent.tools",
        "core.resources",
        "core.pdf",
        "core.models",
        "core.docling",
        "core.platform",
    ]
    .map(|id| json!({"id":id,"version":1,"provider":"core"}));
    let optional = ["core.agent.skills", "core.agent.mcp"]
        .map(|id| json!({"id":id,"version":1,"provider":"core"}));
    value["consumes"] = json!({"required": required, "optional": optional});
    value
}

fn fixture() -> (tempfile::TempDir, Arc<RuntimePluginCatalog>, String, Value) {
    let (root, catalog, _) = tests::literature_fixture(b"host bytes, not frontend execution");
    let package = root.path().join("writing");
    let native: Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    let manifest = combined_manifest(&native);
    fs::write(
        package.join("entry.js"),
        b"throw new Error('must not run');",
    )
    .unwrap();
    fs::write(package.join("style.css"), b"/* fixture style */").unwrap();
    fs::write(
        package.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    (root, catalog, key, manifest)
}

fn validate_value(value: Value, profile: NativeProfile) -> NativeResult<NativeEntry> {
    let manifest = serde_json::from_value(value).map_err(|_| profile_error(profile))?;
    validate_package(manifest, profile)
}

#[test]
fn combined_literature_accept_and_stage_share_the_existing_trace_profile() {
    let (_root, catalog, key, _) = fixture();
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    TraceResources::load(&catalog, &package).unwrap();
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
    assert_eq!(
        fs::read(&staged.executable).unwrap(),
        b"host bytes, not frontend execution"
    );
    assert!(staged.input_directory.is_some());
    assert_eq!(
        accept(&catalog, &key, NativeProfile::WritingPreview)
            .err()
            .unwrap()
            .code,
        "plugin_host_profile_unsupported"
    );
    assert_eq!(
        stage(&catalog, &key).err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}

#[test]
fn combined_metadata_remains_a_closed_footprint_without_frontend_permissions() {
    type Mutation = fn(&mut Value);
    let (_root, _catalog, _key, original) = fixture();
    let mutations: [(&str, Mutation); 30] = [
        ("missing entry", |v| {
            v.as_object_mut().unwrap().remove("entry");
        }),
        ("missing styles", |v| {
            v.as_object_mut().unwrap().remove("styles");
        }),
        ("missing uiRuntime", |v| {
            v.as_object_mut().unwrap().remove("uiRuntime");
        }),
        ("null entry", |v| v["entry"] = Value::Null),
        ("null styles", |v| v["styles"] = Value::Null),
        ("null uiRuntime", |v| v["uiRuntime"] = Value::Null),
        ("unknown field", |v| v["permissions"] = json!(["anything"])),
        ("unknown requirement field", |v| {
            v["consumes"]["required"][0]["extra"] = json!(true)
        }),
        ("native args", |v| {
            v["native"]["args"] = json!(["--anything"])
        }),
        ("native env", |v| {
            v["native"]["env"] = json!({"ANYTHING":"value"})
        }),
        ("extra provides", |v| {
            v["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"core.agent.tools","version":1}))
        }),
        ("duplicate provides", |v| {
            v["provides"][1] = v["provides"][0].clone()
        }),
        ("missing native capability", |v| {
            v["provides"].as_array_mut().unwrap().pop();
        }),
        ("wrong capability version", |v| {
            v["provides"][2]["version"] = json!(2)
        }),
        ("missing required", |v| {
            v["consumes"]["required"].as_array_mut().unwrap().pop();
        }),
        ("extra required", |v| {
            v["consumes"]["required"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"core.git","version":1,"provider":"core"}))
        }),
        ("duplicate required", |v| {
            v["consumes"]["required"][1] = v["consumes"]["required"][0].clone()
        }),
        ("wrong provider", |v| {
            v["consumes"]["required"][0]["provider"] = json!("other")
        }),
        ("missing provider", |v| {
            v["consumes"]["required"][0]
                .as_object_mut()
                .unwrap()
                .remove("provider");
        }),
        ("wrong requirement version", |v| {
            v["consumes"]["required"][0]["version"] = json!(2)
        }),
        ("missing optional", |v| {
            v["consumes"]["optional"].as_array_mut().unwrap().pop();
        }),
        ("duplicate optional", |v| {
            v["consumes"]["optional"][1] = v["consumes"]["optional"][0].clone()
        }),
        ("plugin dependency", |v| {
            v["dependencies"]["required"] = json!([{"id":"other","apiVersion":1}])
        }),
        ("optional plugin dependency", |v| {
            v["dependencies"]["optional"] = json!([{"id":"other","apiVersion":1}])
        }),
        ("other entry", |v| v["entry"] = json!("other.js")),
        ("extra style", |v| {
            v["styles"] = json!(["style.css", "other.css"])
        }),
        ("empty UI metadata", |v| v["uiRuntime"] = json!("")),
        ("padded UI metadata", |v| {
            v["uiRuntime"] = json!(" react@19.2.8")
        }),
        ("wrong API", |v| v["apiVersion"] = json!(2)),
        ("unknown native key", |v| {
            v["native"]["cwd"] = json!("/anything")
        }),
    ];
    for (name, mutate) in mutations {
        let mut value = original.clone();
        mutate(&mut value);
        let error = validate_value(value, NativeProfile::NoteTraceDecision)
            .err()
            .unwrap();
        assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
    }
}

#[test]
fn combined_protocol_and_platform_checks_keep_the_original_error_codes() {
    let (_root, _catalog, _key, original) = fixture();
    for (field, value, code) in [
        (
            "protocolVersion",
            json!(1),
            "plugin_host_protocol_incompatible",
        ),
        (
            "os",
            json!("unsupported"),
            "plugin_host_platform_unsupported",
        ),
        (
            "arch",
            json!("unsupported"),
            "plugin_host_platform_unsupported",
        ),
    ] {
        let mut manifest = original.clone();
        manifest["native"][field] = value;
        assert_eq!(
            validate_value(manifest, NativeProfile::NoteTraceDecision)
                .err()
                .unwrap()
                .code,
            code
        );
    }
}

#[test]
fn frontend_order_and_ui_renderer_metadata_cannot_add_or_remove_native_methods() {
    let (_root, _catalog, _key, mut value) = fixture();
    value["provides"].as_array_mut().unwrap().reverse();
    value["consumes"]["required"]
        .as_array_mut()
        .unwrap()
        .reverse();
    value["uiRuntime"] = json!("different-ui-metadata@1");
    let native = validate_value(value, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(native.protocol_version, 2);
    // The existing Web SDK separately rejects an incompatible UI runtime before
    // importing its ESM. Native acceptance grants only this profile, not UI.
    assert_eq!(
        NativeProfile::NoteTraceDecision.capability(),
        "literature.note-trace-decision"
    );
}

#[test]
fn combined_manifest_cannot_bypass_original_snapshot_resources_or_sql_grants() {
    let (root, catalog, key, _) = fixture();
    fs::write(root.path().join("writing/style.css"), b"changed").unwrap();
    assert_eq!(
        catalog.resource(&key, "style.css").err().unwrap().code,
        "runtime_plugin_resource_changed"
    );
    assert_eq!(
        accept(&catalog, &key, NativeProfile::NoteTraceDecision)
            .err()
            .unwrap()
            .code,
        "plugin_host_package_unavailable"
    );
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    fs::write(
        root.path().join("writing/data/trace-read.v1.sql"),
        b"different bytes",
    )
    .unwrap();
    assert!(TraceResources::load(&catalog, &package).is_err());
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert!(TraceResources::load(&catalog, &package).is_err());
}

#[cfg(unix)]
fn copy_package(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "external fixture must contain regular resources"
        );
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_package(&entry.path(), &destination);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires an explicit independently built combined literature package"]
async fn actual_combined_package_stages_the_original_host_and_handshakes_read_only() {
    let source = std::env::var_os("RESEARCH_ASSISTANT_TEST_COMBINED_LITERATURE_PACKAGE")
        .expect("select an exact independently built combined package");
    let root = tempfile::tempdir().unwrap();
    copy_package(Path::new(&source), &root.path().join("literature"));
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    let key = listing.packages[0].package_key.clone();
    let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    assert_eq!(package.manifest["provides"].as_array().unwrap().len(), 3);
    TraceResources::load(&catalog, &package).unwrap();
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let mut host = NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
    host.initialize().await.unwrap();
    let initialized = host.request_identity().unwrap();
    let decision = host
        .decide_note_trace(
            SnapshotInput {
                query: "combined package synthetic absent trace".into(),
                row: None,
            },
            std::time::Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert_eq!(
        decision,
        TraceDecision::Rejected {
            reason: TraceRejection::NotFound
        }
    );
    let decided = host.request_identity().unwrap();
    assert_eq!(initialized.0, decided.0);
    assert_ne!(initialized.1, decided.1);
    eprintln!(
        "combined native read-only decision: packageKey={key} initialize={initialized:?} decide={decided:?}"
    );
    host.shutdown().await.unwrap();
}

#[test]
fn combined_original_and_confirmation_metadata_keep_the_existing_staging_profile() {
    for confirmation in [false, true] {
        for reverse in [false, true] {
            let (root, catalog, _, mut manifest) = fixture();
            let required = manifest["consumes"]["required"].as_array_mut().unwrap();
            if confirmation {
                required
                    .push(json!({"id":"core.platform.confirmation","version":1,"provider":"core"}));
            }
            if reverse {
                required.reverse();
            }
            assert_eq!(required.len(), if confirmation { 14 } else { 13 });
            fs::write(
                root.path().join("writing/manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            TraceResources::load(&catalog, &package).unwrap();
            let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
            assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
            assert!(staged.input_directory.is_some());
            assert_eq!(
                fs::read(staged.executable).unwrap(),
                b"host bytes, not frontend execution"
            );
            assert_eq!(package.manifest["provides"], manifest["provides"]);
            assert_eq!(
                accept(&catalog, &key, NativeProfile::WritingPreview)
                    .err()
                    .unwrap()
                    .code,
                "plugin_host_profile_unsupported"
            );
        }
    }
}

#[test]
fn combined_confirmation_metadata_cannot_widen_requirements_or_native_permissions() {
    type Mutation = fn(&mut Value);
    let (_, _, _, mut original) = fixture();
    original["consumes"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"core.platform.confirmation","version":1,"provider":"core"}));
    let mutations: [(&str, Mutation); 9] = [
        ("duplicate confirmation", |v| {
            let confirmation = v["consumes"]["required"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .clone();
            v["consumes"]["required"]
                .as_array_mut()
                .unwrap()
                .push(confirmation);
        }),
        ("wrong confirmation provider", |v| {
            v["consumes"]["required"][13]["provider"] = json!("other")
        }),
        ("wrong confirmation version", |v| {
            v["consumes"]["required"][13]["version"] = json!(2)
        }),
        ("missing original dependency", |v| {
            v["consumes"]["required"].as_array_mut().unwrap().remove(0);
        }),
        ("duplicate original dependency", |v| {
            v["consumes"]["required"][1] = v["consumes"]["required"][0].clone()
        }),
        ("unknown required", |v| {
            v["consumes"]["required"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"core.git","version":1,"provider":"core"}));
        }),
        ("confirmation in optional", |v| {
            v["consumes"]["optional"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"core.platform.confirmation","version":1,"provider":"core"}));
        }),
        ("dialog permission", |v| {
            v["permissions"] = json!(["dialog:allow-message"])
        }),
        ("native arguments", |v| v["native"]["args"] = json!([])),
    ];
    for (name, mutate) in mutations {
        let mut value = original.clone();
        mutate(&mut value);
        let error = validate_value(value, NativeProfile::NoteTraceDecision)
            .err()
            .unwrap();
        assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
    }
}

#[test]
fn combined_write_diagnostics_accepts_the_third_exact_set_and_stages_the_same_profile() {
    for reverse in [false, true] {
        let (root, catalog, _, mut manifest) = fixture();
        let required = manifest["consumes"]["required"].as_array_mut().unwrap();
        required.extend([
            json!({"id":"core.platform.confirmation","version":1,"provider":"core"}),
            json!({"id":"core.structured-resources.write-diagnostics","version":1,"provider":"core"}),
        ]);
        assert_eq!(required.len(), 15);
        if reverse {
            required.reverse();
        }
        fs::write(
            root.path().join("writing/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let package = accept(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        TraceResources::load(&catalog, &package).unwrap();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        assert_eq!(staged.profile, NativeProfile::NoteTraceDecision);
        assert!(staged.input_directory.is_some());
        assert_eq!(
            fs::read(staged.executable).unwrap(),
            b"host bytes, not frontend execution"
        );
        assert_eq!(package.manifest["provides"], manifest["provides"]);
    }
}

#[test]
fn combined_write_diagnostics_cannot_widen_the_closed_requirement_footprint() {
    type Mutation = fn(&mut Value);
    let (_, _, _, mut original) = fixture();
    original["consumes"]["required"].as_array_mut().unwrap().extend([
        json!({"id":"core.platform.confirmation","version":1,"provider":"core"}),
        json!({"id":"core.structured-resources.write-diagnostics","version":1,"provider":"core"}),
    ]);
    let mutations: [(&str, Mutation); 7] = [
        ("diagnostics without confirmation", |v| {
            v["consumes"]["required"].as_array_mut().unwrap().remove(13);
        }),
        ("missing original dependency", |v| {
            v["consumes"]["required"].as_array_mut().unwrap().remove(0);
        }),
        ("wrong diagnostics provider", |v| {
            v["consumes"]["required"][14]["provider"] = json!("other");
        }),
        ("wrong diagnostics version", |v| {
            v["consumes"]["required"][14]["version"] = json!(2);
        }),
        ("unknown required", |v| {
            v["consumes"]["required"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"core.git","version":1,"provider":"core"}));
        }),
        ("duplicate diagnostics", |v| {
            v["consumes"]["required"][0] = v["consumes"]["required"][14].clone();
        }),
        ("diagnostics in optional", |v| {
            v["consumes"]["optional"].as_array_mut().unwrap().push(json!({"id":"core.structured-resources.write-diagnostics","version":1,"provider":"core"}));
        }),
    ];
    for (name, mutate) in mutations {
        let mut value = original.clone();
        mutate(&mut value);
        let error = validate_value(value, NativeProfile::NoteTraceDecision)
            .err()
            .unwrap();
        assert_eq!(error.code, "plugin_host_profile_unsupported", "{name}");
    }
}
