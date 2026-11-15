use super::*;
use serde_json::{Value, json};

fn manifest() -> Value {
    json!({
        "formatVersion":2,"id":"writing_projects","version":"1.0.0","apiVersion":1,
        "entry":"entry.js","styles":["style.css"],"uiRuntime":"react@19.2.8/react-dom@19.2.8",
        "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"},
        "dependencies":{"required":[],"optional":[{"id":"literature_library","apiVersion":1}]},
        "provides":[{"id":"writing.projects","version":1},{"id":"writing.note-catalog","version":1},
            {"id":"writing.project-files","version":1},{"id":"writing.import-preview","version":1}],
        "consumes":{"required":writing_combined::REQUIRED.map(|id|json!({"id":id,"version":1,"provider":"core"})),
            "optional":[{"id":"literature.citations","version":1,"provider":"literature_library"},
                {"id":"core.agent.skills","version":1,"provider":"core"},{"id":"core.agent.mcp","version":1,"provider":"core"}]}
    })
}

fn validate(value: Value, profile: NativeProfile) -> NativeResult<NativeEntry> {
    validate_package(
        serde_json::from_value(value).map_err(|_| profile_error(profile))?,
        profile,
    )
}

#[test]
fn combined_writing_has_one_discovered_key_and_keeps_legacy_preview_separate() {
    let (root, catalog, old_key) = tests::fixture(b"host bytes, never execute frontend");
    let directory = root.path().join("writing");
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&manifest()).unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("entry.js"),
        "throw new Error('must not execute');",
    )
    .unwrap();
    fs::write(directory.join("style.css"), "/* synthetic */").unwrap();
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    let key = &listing.packages[0].package_key;
    assert_ne!(key, &old_key);
    let package = accept(&catalog, key, NativeProfile::WritingProjects).unwrap();
    assert_eq!(&package.package_key, key);
    let staged = stage_for(&catalog, key, NativeProfile::WritingProjects).unwrap();
    assert_eq!(
        fs::read(staged.executable).unwrap(),
        b"host bytes, never execute frontend"
    );
    for profile in [
        NativeProfile::WritingPreview,
        NativeProfile::NoteTraceDecision,
    ] {
        assert_eq!(
            accept(&catalog, key, profile).err().unwrap().code,
            "plugin_host_profile_unsupported"
        );
    }
}

#[test]
fn combined_writing_rejects_missing_extra_duplicate_and_foreign_declarations() {
    let valid = manifest();
    validate(valid.clone(), NativeProfile::WritingProjects).unwrap();
    let paths = [
        vec!["provides"],
        vec!["consumes", "required"],
        vec!["consumes", "optional"],
        vec!["dependencies", "optional"],
    ];
    for path in paths {
        for mutation in ["missing", "duplicate", "foreign"] {
            let mut value = valid.clone();
            let mut array = &mut value;
            for part in &path {
                array = &mut array[*part];
            }
            let array = array.as_array_mut().unwrap();
            match mutation {
                "missing" => {
                    array.pop();
                }
                "duplicate" => array.push(array[0].clone()),
                "foreign" => {
                    array[0]["id"] = json!("other");
                }
                _ => unreachable!(),
            }
            assert!(
                validate(value, NativeProfile::WritingProjects).is_err(),
                "{path:?} {mutation}"
            );
        }
    }
    for (path, replacement) in [
        ("/native/protocolVersion", json!(1)),
        ("/provides/0/version", json!(2)),
        ("/consumes/required/0/provider", json!("other")),
        ("/consumes/optional/0/provider", json!("core")),
        ("/dependencies/optional/0/apiVersion", json!(2)),
        ("/entry", json!("other.js")),
        ("/styles/0", json!("other.css")),
        ("/uiRuntime", json!(" invalid ")),
    ] {
        let mut value = valid.clone();
        *value.pointer_mut(path).unwrap() = replacement;
        assert!(
            validate(value, NativeProfile::WritingProjects).is_err(),
            "{path}"
        );
    }
    let mut value = valid;
    value["dependencies"]["required"] = json!([{"id":"other","apiVersion":1}]);
    assert!(validate(value, NativeProfile::WritingProjects).is_err());
}
